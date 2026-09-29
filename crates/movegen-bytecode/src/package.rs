//! On-chain package objects (B-PKG-1): `sui-types` `MovePackage` BCS.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::abi;
use crate::error::Error;
use crate::model::Address;

#[derive(Deserialize)]
struct RawPackage {
    id: [u8; 32],
    version: u64,
    module_map: BTreeMap<String, Vec<u8>>,
    type_origin_table: Vec<RawTypeOrigin>,
    linkage_table: BTreeMap<[u8; 32], RawUpgradeInfo>,
}

#[derive(Deserialize)]
struct RawTypeOrigin {
    module_name: String,
    datatype_name: String,
    package: [u8; 32],
}

#[derive(Deserialize)]
struct RawUpgradeInfo {
    upgraded_id: [u8; 32],
    upgraded_version: u64,
}

/// Where a datatype was first defined.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TypeOrigin {
    /// Module.
    pub module: String,
    /// Datatype.
    pub datatype: String,
    /// The package version (storage id) that introduced it: its defining id.
    pub defining_id: String,
}

/// Which version of a dependency this package links against.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Linkage {
    /// The dependency's original id.
    pub original_id: String,
    /// The storage id of the linked version.
    pub upgraded_id: String,
    /// The linked version.
    pub version: u64,
}

/// A package object: storage id, version, module bytecode, type origins and linkage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    /// Storage id (this version's object id).
    pub id: Address,
    /// Object version.
    pub version: u64,
    /// Module name → bytecode.
    pub modules: BTreeMap<String, Vec<u8>>,
    /// Type origins.
    pub type_origins: Vec<TypeOrigin>,
    /// Linkage table, by dependency original id.
    pub linkage: Vec<Linkage>,
}

impl Package {
    /// Decodes `MovePackage` BCS (e.g. GraphQL `packageBcs`); trailing bytes are rejected.
    pub fn from_bcs(bytes: &[u8]) -> Result<Self, Error> {
        let raw: RawPackage =
            bcs::from_bytes(bytes).map_err(|e| Error::BadPackage(e.to_string()))?;
        Ok(Package {
            id: Address(raw.id),
            version: raw.version,
            modules: raw.module_map,
            type_origins: raw
                .type_origin_table
                .into_iter()
                .map(|t| TypeOrigin {
                    module: t.module_name,
                    datatype: t.datatype_name,
                    defining_id: Address(t.package).to_hex(),
                })
                .collect(),
            linkage: raw
                .linkage_table
                .into_iter()
                .map(|(original, info)| Linkage {
                    original_id: Address(original).to_hex(),
                    upgraded_id: Address(info.upgraded_id).to_hex(),
                    version: info.upgraded_version,
                })
                .collect(),
        })
    }

    /// Every module's ABI, in module-name order.
    pub fn module_abis(&self) -> Result<Vec<abi::Module>, Error> {
        self.modules
            .iter()
            .map(|(name, bytes)| {
                abi::Module::from_bytes(bytes).map_err(|e| Error::InModule {
                    module: name.clone(),
                    source: Box::new(e),
                })
            })
            .collect()
    }
}
