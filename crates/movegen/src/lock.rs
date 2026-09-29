//! The lock: everything generation needs, committed next to the config (L rules).
//!
//! ```text
//! movegen.lock/
//!   deployments/<network>.json      ids, versions, type origins, linkage, system objects
//!   abi/<network>/<package>.json    symbolized ABI: every package address is `@name`
//! ```
//!
//! ABIs contain no network-specific ids, so the same package on two networks diffs cleanly.
//! Files are pretty-printed JSON with a trailing newline, written only when they change.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use movegen_bytecode::abi;
use serde::{Deserialize, Serialize};

use crate::Error;

/// Lock format version.
pub const LOCK_VERSION: u32 = 1;

/// A whole lock.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lock {
    /// Per-network contents.
    pub networks: BTreeMap<String, NetworkLock>,
}

/// One network's part of the lock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkLock {
    /// Deployment data.
    pub deployment: Deployment,
    /// Symbolized ABIs by package symbol.
    pub packages: BTreeMap<String, PackageAbi>,
}

/// A package's symbolized ABI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageAbi {
    /// Package symbol.
    pub name: String,
    /// Modules in name order; addresses are `@symbol`s.
    pub modules: Vec<abi::Module>,
}

/// A network's deployment data (L-DEP-1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deployment {
    /// Lock format version.
    pub lock_version: u32,
    /// Network name.
    pub network: String,
    /// Chain identifier (genesis checkpoint digest).
    pub chain_identifier: String,
    /// Checkpoint the data was read at.
    pub checkpoint: u64,
    /// Well-known shared objects by name.
    pub system_objects: BTreeMap<String, SystemObject>,
    /// Packages by symbol.
    pub packages: BTreeMap<String, PackageDeployment>,
}

/// A well-known shared object.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemObject {
    /// Object id.
    pub id: String,
    /// Initial shared version (differs per network for most system objects).
    pub initial_shared_version: u64,
}

/// A package's ids on one network.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageDeployment {
    /// Original id: the runtime address in bytecode, and the default type origin.
    pub original_id: String,
    /// Storage id of the latest version: where calls go.
    pub published_at: String,
    /// Latest version.
    pub version: u64,
    /// Where each datatype was first defined (`module::Datatype` → defining id).
    pub type_origins: BTreeMap<String, String>,
    /// Linked dependency versions by dependency symbol.
    pub linkage: BTreeMap<String, LinkedVersion>,
}

/// One linkage entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedVersion {
    /// Storage id of the linked version.
    pub upgraded_id: String,
    /// Linked version.
    pub version: u64,
}

fn to_json<T: Serialize>(value: &T) -> String {
    let mut text = serde_json::to_string_pretty(value).expect("lock types serialize");
    text.push('\n');
    text
}

/// Writes `contents` to `path` only if it differs; returns whether it changed.
fn write_if_changed(path: &Path, contents: &str) -> Result<bool, Error> {
    if std::fs::read_to_string(path).is_ok_and(|old| old == contents) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, &e))?;
    }
    std::fs::write(path, contents).map_err(|e| Error::io(path, &e))?;
    Ok(true)
}

impl Lock {
    /// The files this lock consists of, as `(relative path, contents)`, in path order.
    pub fn files(&self) -> Vec<(PathBuf, String)> {
        let mut files = Vec::new();
        for (network, part) in &self.networks {
            files.push((
                PathBuf::from(format!("deployments/{network}.json")),
                to_json(&part.deployment),
            ));
            for (name, abi) in &part.packages {
                files.push((
                    PathBuf::from(format!("abi/{network}/{name}.json")),
                    to_json(abi),
                ));
            }
        }
        files.sort();
        files
    }

    /// Keeps `previous`'s checkpoint for every network whose deployment and ABIs are otherwise
    /// unchanged, so refreshing an up-to-date lock changes no files (L-DET-2).
    pub fn stabilize(&mut self, previous: &Lock) {
        for (network, part) in &mut self.networks {
            let Some(old) = previous.networks.get(network) else {
                continue;
            };
            let mut same = part.deployment.clone();
            same.checkpoint = old.deployment.checkpoint;
            if same == old.deployment && part.packages == old.packages {
                part.deployment.checkpoint = old.deployment.checkpoint;
            }
        }
    }

    /// Writes the lock under `dir`, removing stale files it owns. Returns changed paths.
    pub fn write(&self, dir: &Path) -> Result<Vec<PathBuf>, Error> {
        let files = self.files();
        let mut changed = Vec::new();
        for (rel, contents) in &files {
            if write_if_changed(&dir.join(rel), contents)? {
                changed.push(rel.clone());
            }
        }
        let keep: std::collections::BTreeSet<_> = files.iter().map(|(p, _)| p.clone()).collect();
        for rel in existing_files(dir)? {
            if !keep.contains(&rel) {
                std::fs::remove_file(dir.join(&rel)).map_err(|e| Error::io(&dir.join(&rel), &e))?;
                changed.push(rel);
            }
        }
        Ok(changed)
    }

    /// Reads a lock from `dir`.
    pub fn read(dir: &Path) -> Result<Self, Error> {
        let mut lock = Lock::default();
        let read = |rel: &Path| -> Result<String, Error> {
            let path = dir.join(rel);
            std::fs::read_to_string(&path).map_err(|e| Error::io(&path, &e))
        };
        let mut files = existing_files(dir)?;
        files.sort();
        for rel in &files {
            let parts: Vec<_> = rel
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            if let [kind, file] = parts.as_slice()
                && kind == "deployments"
            {
                let network = file.trim_end_matches(".json").to_owned();
                let deployment: Deployment = serde_json::from_str(&read(rel)?)
                    .map_err(|e| Error::Lock(format!("{}: {e}", rel.display())))?;
                if deployment.lock_version != LOCK_VERSION {
                    return Err(Error::Lock(format!(
                        "{}: lock version {} (expected {LOCK_VERSION}); run `movegen refresh`",
                        rel.display(),
                        deployment.lock_version
                    )));
                }
                lock.networks.insert(
                    network,
                    NetworkLock {
                        deployment,
                        packages: BTreeMap::new(),
                    },
                );
            }
        }
        for rel in &files {
            let parts: Vec<_> = rel
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            if let [kind, network, file] = parts.as_slice()
                && kind == "abi"
            {
                let abi: PackageAbi = serde_json::from_str(&read(rel)?)
                    .map_err(|e| Error::Lock(format!("{}: {e}", rel.display())))?;
                let part = lock.networks.get_mut(network).ok_or_else(|| {
                    Error::Lock(format!("{}: no deployments/{network}.json", rel.display()))
                })?;
                if abi.name != file.trim_end_matches(".json") {
                    return Err(Error::Lock(format!(
                        "{}: names package {:?}",
                        rel.display(),
                        abi.name
                    )));
                }
                part.packages.insert(abi.name.clone(), abi);
            }
        }
        Ok(lock)
    }
}

/// `.json` files under `dir/deployments` and `dir/abi/*`, relative to `dir`.
fn existing_files(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut out = Vec::new();
    let list = |sub: &Path, out: &mut Vec<PathBuf>| -> Result<(), Error> {
        let full = dir.join(sub);
        let Ok(entries) = std::fs::read_dir(&full) else {
            return Ok(());
        };
        for entry in entries {
            let entry = entry.map_err(|e| Error::io(&full, &e))?;
            let name = entry.file_name();
            if entry.path().is_file() && name.to_string_lossy().ends_with(".json") {
                out.push(sub.join(name));
            }
        }
        Ok(())
    };
    list(Path::new("deployments"), &mut out)?;
    if let Ok(networks) = std::fs::read_dir(dir.join("abi")) {
        for network in networks.flatten() {
            if network.path().is_dir() {
                list(&Path::new("abi").join(network.file_name()), &mut out)?;
            }
        }
    }
    Ok(out)
}
