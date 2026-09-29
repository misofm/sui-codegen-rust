//! `movegen`: refresh a lock of Sui Move package ABIs, and generate Rust bindings from it.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use base64::Engine as _;
use clap::{Parser, Subcommand};
use movegen::refresh::{ChainInfo, FetchedPackage, SYSTEM_OBJECTS, Source};
use movegen::{Config, Error, compare_networks};
use movegen_bytecode::model::Address;
use serde_json::{Value, json};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Path to movegen.toml.
    #[arg(long, default_value = "movegen.toml", global = true)]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch every configured package (and its dependency closure) into the lock.
    Refresh,
    /// Generate Rust bindings from the lock (offline).
    Generate {
        /// Fail if the generated files are not up to date instead of writing them.
        #[arg(long)]
        check: bool,
    },
}

/// Sui GraphQL, batched: one `multiGetPackages` per dependency level (≤ 200 keys per request).
struct GraphQl<'c> {
    config: &'c Config,
    agent: ureq::Agent,
}

const MAX_MULTI_GET: usize = 200;

impl GraphQl<'_> {
    fn query(&self, network: &str, query: &str, variables: &Value) -> Result<Value, Error> {
        let url = &self.config.networks[network].graphql;
        let body: Value = self
            .agent
            .post(url)
            .header("user-agent", concat!("movegen/", env!("CARGO_PKG_VERSION")))
            .send_json(json!({ "query": query, "variables": variables }))
            .map_err(|e| Error::Fetch(format!("{network}: {e}")))?
            .body_mut()
            .read_json()
            .map_err(|e| Error::Fetch(format!("{network}: {e}")))?;
        if let Some(errors) = body.get("errors") {
            return Err(Error::Fetch(format!("{network}: {errors}")));
        }
        Ok(body["data"].clone())
    }
}

impl Source for GraphQl<'_> {
    fn chain(&self, network: &str) -> Result<ChainInfo, Error> {
        let keys: Vec<Value> = SYSTEM_OBJECTS
            .iter()
            .map(|(id, _)| json!({ "address": id }))
            .collect();
        let data = self.query(
            network,
            "query($keys: [ObjectKey!]!) { chainIdentifier checkpoint { sequenceNumber } \
             multiGetObjects(keys: $keys) { address owner { __typename ... on Shared { initialSharedVersion } } } }",
            &json!({ "keys": keys }),
        )?;
        let bad = |what: &str| Error::Fetch(format!("{network}: malformed {what}"));
        Ok(ChainInfo {
            chain_identifier: data["chainIdentifier"]
                .as_str()
                .ok_or_else(|| bad("chainIdentifier"))?
                .to_owned(),
            checkpoint: data["checkpoint"]["sequenceNumber"]
                .as_u64()
                .ok_or_else(|| bad("checkpoint"))?,
            system_objects: data["multiGetObjects"]
                .as_array()
                .ok_or_else(|| bad("objects"))?
                .iter()
                .filter(|o| o["owner"]["__typename"] == "Shared")
                .filter_map(|o| {
                    Some((
                        Address::from_hex(o["address"].as_str()?)?,
                        o["owner"]["initialSharedVersion"].as_u64()?,
                    ))
                })
                .collect(),
        })
    }

    fn packages(&self, network: &str, addresses: &[Address]) -> Result<Vec<FetchedPackage>, Error> {
        let mut out = Vec::with_capacity(addresses.len());
        for chunk in addresses.chunks(MAX_MULTI_GET) {
            let keys: Vec<Value> = chunk
                .iter()
                .map(|a| json!({ "address": a.to_hex() }))
                .collect();
            let data = self.query(
                network,
                "query($keys: [PackageKey!]!) { multiGetPackages(keys: $keys) { address version packageBcs } }",
                &json!({ "keys": keys }),
            )?;
            let results = data["multiGetPackages"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            for (requested, p) in chunk
                .iter()
                .zip(results.iter().chain(std::iter::repeat(&Value::Null)))
            {
                if p.is_null() {
                    return Err(Error::Fetch(format!(
                        "{network}: package {requested} not found"
                    )));
                }
                let bad = || Error::Fetch(format!("{network}: malformed package {requested}"));
                out.push(FetchedPackage {
                    storage_id: Address::from_hex(p["address"].as_str().ok_or_else(bad)?)
                        .ok_or_else(bad)?,
                    version: p["version"].as_u64().ok_or_else(bad)?,
                    bcs: base64::engine::general_purpose::STANDARD
                        .decode(p["packageBcs"].as_str().ok_or_else(bad)?)
                        .map_err(|_| bad())?,
                });
            }
        }
        Ok(out)
    }
}

fn run(cli: &Cli) -> Result<(), Error> {
    let (config, dir) = Config::load(&cli.config)?;
    match cli.command {
        Command::Refresh => {
            let start = Instant::now();
            let source = GraphQl {
                config: &config,
                agent: ureq::Agent::new_with_defaults(),
            };
            let mut lock = movegen::refresh(&config, &source)?;
            let lock_dir = dir.join(&config.lock);
            if let Ok(previous) = movegen::Lock::read(&lock_dir) {
                lock.stabilize(&previous);
            }
            let changed = lock.write(&lock_dir)?;
            for (network, part) in &lock.networks {
                eprintln!(
                    "{network}: {} packages at checkpoint {}",
                    part.packages.len(),
                    part.deployment.checkpoint
                );
                if *network != config.reference_network {
                    for d in compare_networks(&lock, &config.reference_network, network) {
                        eprintln!("  warning: {}::{} {}", d.package, d.module, d.detail);
                    }
                }
            }
            eprintln!(
                "{} files changed in {} ({:.2?})",
                changed.len(),
                lock_dir.display(),
                start.elapsed()
            );
            Ok(())
        }
        Command::Generate { check } => {
            let start = Instant::now();
            let lock = movegen::Lock::read(&dir.join(&config.lock))?;
            let generated = movegen::generate(&config, &lock)?;
            let out = dir.join(
                config
                    .generate
                    .output
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("src/generated")),
            );
            let changes = generated.sync(&out, check)?;
            let lines: usize = generated.files.iter().map(|(_, c)| c.lines().count()).sum();
            if check && !changes.is_empty() {
                for change in &changes {
                    match change {
                        movegen::Change::Write(rel) => {
                            eprintln!("stale: {}", out.join(rel).display());
                        }
                        movegen::Change::Remove(rel) => {
                            eprintln!("orphaned: {}", out.join(rel).display());
                        }
                    }
                }
                return Err(Error::Generate(format!(
                    "{} generated files are out of date; run `movegen generate`",
                    changes.len()
                )));
            }
            eprintln!(
                "{} files ({lines} lines), {} changed, in {} ({:.2?})",
                generated.files.len(),
                if check { 0 } else { changes.len() },
                out.display(),
                start.elapsed()
            );
            Ok(())
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
