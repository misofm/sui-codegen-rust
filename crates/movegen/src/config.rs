//! `movegen.toml`: which packages to bind, on which networks.
//!
//! ```toml
//! reference_network = "mainnet"      # the network generation reads ABIs from
//! lock = "movegen.lock"              # lock directory, relative to this file
//!
//! [networks.mainnet]
//! graphql = "https://graphql.mainnet.sui.io/graphql"
//!
//! [packages.record_shop]
//! mainnet = "0x375b…"                # any id in the package's lineage
//! testnet = "0x6eb6…"
//! ```
//!
//! Dependencies are discovered through linkage and need no entry; give one a name by listing it
//! here (the name becomes its `@symbol` in the lock).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Error;

/// A parsed `movegen.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The network whose ABIs bindings are generated from.
    pub reference_network: String,
    /// Lock directory (default `movegen.lock`), relative to the config file.
    #[serde(default = "default_lock")]
    pub lock: PathBuf,
    /// Networks by name.
    pub networks: BTreeMap<String, NetworkConfig>,
    /// Packages by symbol name; each maps network → an id in the package's lineage.
    pub packages: BTreeMap<String, BTreeMap<String, String>>,
    /// Generation options.
    #[serde(default)]
    pub generate: GenerateConfig,
}

/// `[generate]`: what to emit and where.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateConfig {
    /// Output directory for the generated module tree (default `src/generated`), relative to
    /// the config file.
    #[serde(default)]
    pub output: Option<PathBuf>,
    /// Packages (`name`) or modules (`name::module`) whose public/entry functions get call
    /// builders. Default: every configured package.
    #[serde(default)]
    pub functions: Option<Vec<String>>,
}

/// One network.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkConfig {
    /// Sui GraphQL endpoint.
    pub graphql: String,
}

fn default_lock() -> PathBuf {
    PathBuf::from("movegen.lock")
}

impl Config {
    /// Parses and validates a config.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let config: Config = toml::from_str(text).map_err(|e| Error::Config(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    /// Reads `path`, returning the config and the directory it is relative to.
    pub fn load(path: &Path) -> Result<(Self, PathBuf), Error> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, &e))?;
        let dir = path
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        Ok((Self::parse(&text)?, dir))
    }

    fn validate(&self) -> Result<(), Error> {
        if !self.networks.contains_key(&self.reference_network) {
            return Err(Error::Config(format!(
                "reference_network {:?} is not in [networks]",
                self.reference_network
            )));
        }
        for (name, ids) in &self.packages {
            if !is_symbol(name) {
                return Err(Error::Config(format!(
                    "package name {name:?} must be a lowercase identifier ([a-z][a-z0-9_]*)"
                )));
            }
            for (network, id) in ids {
                if !self.networks.contains_key(network) {
                    return Err(Error::Config(format!(
                        "[packages.{name}] names unknown network {network:?}"
                    )));
                }
                if movegen_bytecode::model::Address::from_hex(id).is_none() {
                    return Err(Error::Config(format!(
                        "[packages.{name}] {network} = {id:?} is not an address"
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Package symbols are lowercase identifiers: they become Rust module names.
pub(crate) fn is_symbol(s: &str) -> bool {
    let b = s.as_bytes();
    b.first().is_some_and(u8::is_ascii_lowercase)
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_validates() {
        let text = r#"
            reference_network = "mainnet"
            [networks.mainnet]
            graphql = "https://graphql.mainnet.sui.io/graphql"
            [packages.record_shop]
            mainnet = "0x375b"
        "#;
        let config = Config::parse(text).unwrap();
        assert_eq!(config.lock, PathBuf::from("movegen.lock"));
        assert!(Config::parse(&text.replace("record_shop", "RecordShop")).is_err());
        assert!(Config::parse(&text.replace("mainnet = \"0x375b\"", "devnet = \"0x1\"")).is_err());
        assert!(Config::parse(&text.replace("0x375b", "zz")).is_err());
        assert!(
            Config::parse(&text.replace(
                "reference_network = \"mainnet\"",
                "reference_network = \"x\""
            ))
            .is_err()
        );
    }
}
