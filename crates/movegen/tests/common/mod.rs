//! Shared test support: the fixture corpus as a `Source`, and a config covering it.
#![allow(dead_code, unreachable_pub, missing_docs)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use movegen::refresh::{ChainInfo, FetchedPackage, Source};
use movegen::{Config, Error};
use movegen_bytecode::model::Address;
use serde_json::Value;

pub fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/packages")
}

/// Serves `fixtures/packages/<network>/` as recorded by `scripts/fetch-fixtures.py`.
pub struct Fixtures;

impl Source for Fixtures {
    fn chain(&self, network: &str) -> Result<ChainInfo, Error> {
        let v: Value = serde_json::from_str(
            &std::fs::read_to_string(fixtures().join(network).join("chain.json")).unwrap(),
        )
        .unwrap();
        Ok(ChainInfo {
            chain_identifier: v["chain_identifier"].as_str().unwrap().to_owned(),
            checkpoint: v["checkpoint"].as_u64().unwrap(),
            system_objects: v["system_objects"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (Address::from_hex(k).unwrap(), v.as_u64().unwrap()))
                .collect(),
        })
    }

    fn packages(&self, network: &str, addresses: &[Address]) -> Result<Vec<FetchedPackage>, Error> {
        addresses
            .iter()
            .map(|a| Fixtures::package(network, a))
            .collect()
    }
}

impl Fixtures {
    fn package(network: &str, address: &Address) -> Result<FetchedPackage, Error> {
        let dir = fixtures().join(network);
        let manifest: Vec<Value> =
            serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap())
                .unwrap();
        let wanted = address.to_hex();
        let storage = manifest
            .iter()
            .find(|m| {
                Address::from_hex(m["requested"].as_str().unwrap())
                    .unwrap()
                    .to_hex()
                    == wanted
                    || m["storage_id"].as_str().unwrap() == wanted
            })
            .map(|m| m["storage_id"].as_str().unwrap().to_owned())
            .ok_or_else(|| Error::Fetch(format!("{wanted} not in fixtures")))?;
        let bcs = std::fs::read(dir.join(format!("{storage}.bcs"))).unwrap();
        let package = movegen_bytecode::Package::from_bcs(&bcs).unwrap();
        Ok(FetchedPackage {
            storage_id: package.id,
            version: package.version,
            bcs,
        })
    }
}

/// A readable package name from an SDK deployment key, e.g. `recordSales.recordShopPackageId`
/// → `record_shop`, `operations.actions.partyWallet` → `party_wallet_action`.
pub fn package_name(key: &str) -> String {
    let parts: Vec<&str> = key.trim_start_matches("miso.").split('.').collect();
    let last = parts[parts.len() - 1];
    let base = match last {
        "packageId" => parts[parts.len() - 2],
        other => other.strip_suffix("PackageId").unwrap_or(other),
    };
    let mut name: String = base
        .chars()
        .flat_map(|c| {
            if c.is_ascii_uppercase() {
                vec!['_', c.to_ascii_lowercase()]
            } else {
                vec![c]
            }
        })
        .collect();
    if parts.len() >= 3 && parts[0] == "operations" {
        match parts[1] {
            "actions" => name.push_str("_action"),
            "plugins" => name.push_str("_plugin"),
            _ => {}
        }
    }
    name
}

pub fn miso_config() -> Config {
    let corpus: Value =
        serde_json::from_str(&std::fs::read_to_string(fixtures().join("../corpus.json")).unwrap())
            .unwrap();
    let mut packages: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for network in ["mainnet", "testnet"] {
        let mut seen = std::collections::BTreeSet::new();
        for (key, id) in corpus[network].as_object().unwrap() {
            // Only packages (the registry id is an object); the SDK lists some packages twice.
            if key.starts_with("miso.")
                && !key.contains("registryId")
                && seen.insert(id.as_str().unwrap())
            {
                packages
                    .entry(package_name(key))
                    .or_default()
                    .insert(network.to_owned(), id.as_str().unwrap().to_owned());
            }
        }
    }
    let mut config = Config::parse(
        r#"
        reference_network = "mainnet"
        [networks.mainnet]
        graphql = "https://graphql.mainnet.sui.io/graphql"
        [networks.testnet]
        graphql = "https://graphql.testnet.sui.io/graphql"
        [packages]
        "#,
    )
    .unwrap();
    config.packages = packages;
    config
}
