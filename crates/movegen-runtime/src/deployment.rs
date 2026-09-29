//! Per-network package ids, type origins and system objects (the lock's `deployments/*.json`).
//!
//! Generated code never contains a package id: every call and type tag reads it from a
//! [`Deployment`], which is plain data — load it from the lock, or build it by hand.

use std::collections::BTreeMap;

use serde::Deserialize;
use sui_sdk_types::Address;

use crate::Error;

/// One package on one network.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct PackageIds {
    /// Original id (runtime address in bytecode; default type origin).
    pub original_id: Address,
    /// Latest storage id: where calls go.
    pub published_at: Address,
    /// `module::Datatype` → defining id, for types added by upgrades.
    #[serde(default)]
    pub type_origins: BTreeMap<String, Address>,
}

impl PackageIds {
    /// A package that was never upgraded (or whose types all originate at `id`): calls and type
    /// tags both use `id`.
    pub fn at(id: Address) -> Self {
        Self {
            original_id: id,
            published_at: id,
            type_origins: BTreeMap::new(),
        }
    }
}

/// A well-known shared object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct SystemObject {
    /// Object id.
    pub id: Address,
    /// Initial shared version on this network.
    pub initial_shared_version: u64,
}

/// Everything bindings need to know about one network.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Deployment {
    /// Network name.
    pub network: String,
    /// Chain identifier.
    pub chain_identifier: String,
    /// Packages by symbol.
    pub packages: BTreeMap<String, PackageIds>,
    /// System objects by name (`clock`, `random`, `deny_list`, …).
    #[serde(default)]
    pub system_objects: BTreeMap<String, SystemObject>,
}

impl Deployment {
    /// An empty deployment, to fill with [`Deployment::with_package`] and
    /// [`Deployment::with_system_object`] (for callers that know their ids without a lock).
    pub fn new(network: impl Into<String>, chain_identifier: impl Into<String>) -> Self {
        Self {
            network: network.into(),
            chain_identifier: chain_identifier.into(),
            packages: BTreeMap::new(),
            system_objects: BTreeMap::new(),
        }
    }

    /// Adds (or replaces) a package.
    #[must_use]
    pub fn with_package(mut self, symbol: &str, ids: PackageIds) -> Self {
        // `insert`, not `collect`: building maps from iterators pulls a sort into wasm builds.
        self.packages.insert(symbol.to_owned(), ids);
        self
    }

    /// Adds (or replaces) a system object.
    #[must_use]
    pub fn with_system_object(mut self, name: &str, object: SystemObject) -> Self {
        self.system_objects.insert(name.to_owned(), object);
        self
    }

    /// Parses a lock `deployments/<network>.json` (extra fields are ignored).
    pub fn from_lock_json(json: &str) -> Result<Self, Error> {
        serde_json::from_str(json).map_err(|e| Error::Deployment(e.to_string()))
    }

    /// A package's ids.
    pub fn package(&self, symbol: &'static str) -> Result<&PackageIds, Error> {
        self.packages
            .get(symbol)
            .ok_or(Error::MissingPackage(symbol))
    }

    /// Where calls into `symbol` go.
    pub fn published_at(&self, symbol: &'static str) -> Result<Address, Error> {
        Ok(self.package(symbol)?.published_at)
    }

    /// The defining id of `module::name` in package `symbol` (T-TAG-1).
    pub fn defining_id(
        &self,
        symbol: &'static str,
        module: &str,
        name: &str,
    ) -> Result<Address, Error> {
        let package = self.package(symbol)?;
        let key = format!("{module}::{name}");
        Ok(package
            .type_origins
            .get(&key)
            .copied()
            .unwrap_or(package.original_id))
    }

    /// A system object by name.
    pub fn system_object(&self, name: &'static str) -> Result<SystemObject, Error> {
        self.system_objects
            .get(name)
            .copied()
            .ok_or(Error::MissingSystemObject(name))
    }

    /// Fails unless every package in `required` is present (generated crates export theirs).
    pub fn require(&self, required: &[&'static str]) -> Result<(), Error> {
        required
            .iter()
            .try_for_each(|p| self.package(p).map(|_| ()))
    }
}
