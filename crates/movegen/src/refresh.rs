//! `refresh`: read packages from a network into the lock (L rules in docs/SPEC.md).
//!
//! Starting from the configured packages, every package's linkage table is followed so the lock
//! holds the whole dependency closure. Each package is read at the latest version of its lineage.
//! Addresses in ABIs are then *symbolized*: any id belonging to a package's lineage — its
//! original id, its storage id, any type-origin defining id, any linked version — becomes
//! `@name`. An address outside the closure is an error, never a silent leak of an id into the
//! network-agnostic ABI.

use std::collections::BTreeMap;

use movegen_bytecode::Package;
use movegen_bytecode::abi::{self, DatatypeKind, Type};
use movegen_bytecode::model::Address;

use crate::Error;
use crate::config::Config;
use crate::lock::{
    Deployment, LOCK_VERSION, LinkedVersion, Lock, NetworkLock, PackageAbi, PackageDeployment,
    SystemObject,
};

/// A package as fetched: its storage id, version and `MovePackage` BCS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedPackage {
    /// Storage id of the returned version.
    pub storage_id: Address,
    /// Version.
    pub version: u64,
    /// `MovePackage` BCS.
    pub bcs: Vec<u8>,
}

/// Chain identity and well-known shared objects at a checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainInfo {
    /// Chain identifier.
    pub chain_identifier: String,
    /// Checkpoint.
    pub checkpoint: u64,
    /// Initial shared versions of well-known objects, by id.
    pub system_objects: BTreeMap<Address, u64>,
}

/// Where package data comes from (a GraphQL endpoint in the CLI, fixtures in tests).
pub trait Source {
    /// Chain identity and system objects.
    fn chain(&self, network: &str) -> Result<ChainInfo, Error>;
    /// The latest version in the lineage of each address (any id in a lineage), in order.
    /// Implementations should batch: `refresh` asks once per level of the dependency graph.
    fn packages(&self, network: &str, addresses: &[Address]) -> Result<Vec<FetchedPackage>, Error>;
}

/// Well-known shared objects, by id, with their lock names (L-DEP-2).
pub const SYSTEM_OBJECTS: [(&str, &str); 8] = [
    ("0x5", "sui_system_state"),
    ("0x6", "clock"),
    ("0x8", "random"),
    ("0xa", "address_alias_state"),
    ("0xc", "coin_registry"),
    ("0xd", "display_registry"),
    ("0x403", "deny_list"),
    ("0xacc", "accumulator_root"),
];

/// Names for system packages discovered through linkage (L-SYM-2).
const SYSTEM_PACKAGES: [(&str, &str); 5] = [
    ("0x1", "std"),
    ("0x2", "sui"),
    ("0x3", "sui_system"),
    ("0xb", "bridge"),
    ("0xdee9", "deepbook_v2"),
];

fn addr(s: &str) -> Address {
    Address::from_hex(s).expect("static address")
}

/// Refreshes every configured network, in parallel.
pub fn refresh(config: &Config, source: &(dyn Source + Sync)) -> Result<Lock, Error> {
    let results: Vec<(String, Result<NetworkLock, Error>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = config
            .networks
            .keys()
            .map(|network| {
                (
                    network.clone(),
                    scope.spawn(move || refresh_network(config, network, source)),
                )
            })
            .collect();
        handles
            .into_iter()
            .map(|(network, h)| {
                (
                    network,
                    h.join()
                        .unwrap_or_else(|_| Err(Error::Fetch("refresh panicked".into()))),
                )
            })
            .collect()
    });
    let mut lock = Lock::default();
    for (network, result) in results {
        lock.networks.insert(network, result?);
    }
    Ok(lock)
}

struct Fetched {
    package: Package,
    modules: Vec<abi::Module>,
}

/// Refreshes one network (L-CLO-1, L-SYM-1).
pub fn refresh_network(
    config: &Config,
    network: &str,
    source: &dyn Source,
) -> Result<NetworkLock, Error> {
    let mut names: BTreeMap<Address, String> = BTreeMap::new();
    let mut fetched: BTreeMap<Address, Fetched> = BTreeMap::new();
    let mut level: Vec<(Option<String>, Address)> = config
        .packages
        .iter()
        .filter_map(|(name, ids)| ids.get(network).map(|id| (Some(name.clone()), addr(id))))
        .collect();
    // Every id seen in a fetched lineage → its original id, so known lineages are not refetched.
    let mut lineages: BTreeMap<Address, Address> = BTreeMap::new();

    // L-CLO-1: follow linkage level by level (one batched fetch per level) until closed.
    while !level.is_empty() {
        let wanted: Vec<Address> = {
            let mut w: Vec<Address> = level
                .iter()
                .map(|(_, a)| *a)
                .filter(|a| !lineages.contains_key(a))
                .collect();
            w.sort();
            w.dedup();
            w
        };
        let mut next = Vec::new();
        let results = source.packages(network, &wanted)?;
        if results.len() != wanted.len() {
            return Err(Error::Fetch(format!(
                "asked for {} packages, got {}",
                wanted.len(),
                results.len()
            )));
        }
        for (requested, raw) in wanted.iter().zip(results) {
            let package = Package::from_bcs(&raw.bcs)
                .map_err(|e| Error::Package(format!("{}: {e}", raw.storage_id)))?;
            let modules = package
                .module_abis()
                .map_err(|e| Error::Package(format!("{}: {e}", raw.storage_id)))?;
            let original = original_id(&modules, &raw.storage_id)?;
            lineages.insert(*requested, original);
            lineages.insert(original, original);
            lineages.insert(package.id, original);
            for o in &package.type_origins {
                lineages.insert(addr(&o.defining_id), original);
            }
            if fetched.contains_key(&original) {
                continue;
            }
            for link in &package.linkage {
                next.push((None, addr(&link.upgraded_id)));
            }
            fetched.insert(original, Fetched { package, modules });
        }
        for (name, address) in level {
            let Some(name) = name else { continue };
            let original = *lineages
                .get(&address)
                .expect("every requested id was fetched");
            match names.get(&original) {
                Some(existing) if *existing != name => {
                    return Err(Error::Config(format!(
                        "packages {existing:?} and {name:?} are the same package on {network}"
                    )));
                }
                _ => {
                    names.insert(original, name);
                }
            }
        }
        level = next;
    }

    // L-SYM-2: unnamed dependencies are named after their first module, which (unlike ids) is
    // the same on every network. Collisions must be resolved by naming the package in config.
    let system: BTreeMap<Address, &str> =
        SYSTEM_PACKAGES.iter().map(|(a, n)| (addr(a), *n)).collect();
    for (original, f) in &fetched {
        if !names.contains_key(original) {
            let name = system.get(original).map_or_else(
                || {
                    format!(
                        "pkg_{}",
                        f.modules.first().map_or("empty", |m| m.name.as_str())
                    )
                },
                |n| (*n).to_owned(),
            );
            names.insert(*original, name);
        }
    }
    let mut owners: BTreeMap<&str, Address> = BTreeMap::new();
    for (original, name) in &names {
        if let Some(other) = owners.insert(name, *original) {
            return Err(Error::Config(format!(
                "{other} and {original} would both be named {name:?} on {network}; \
                 name one of them in [packages]"
            )));
        }
    }

    // L-SYM-1: every id of every lineage → `@name`.
    let mut symbols: BTreeMap<String, String> = BTreeMap::new();
    for (original, f) in &fetched {
        let symbol = format!("@{}", names[original]);
        symbols.insert(original.to_hex(), symbol.clone());
        symbols.insert(f.package.id.to_hex(), symbol.clone());
        for origin in &f.package.type_origins {
            symbols.insert(origin.defining_id.clone(), symbol.clone());
        }
    }
    for f in fetched.values() {
        for link in &f.package.linkage {
            if let Some(symbol) = symbols.get(&link.original_id).cloned() {
                symbols.insert(link.upgraded_id.clone(), symbol);
            }
        }
    }

    let mut packages = BTreeMap::new();
    let mut deployments = BTreeMap::new();
    for (original, f) in fetched {
        let name = names[&original].clone();
        let modules = f
            .modules
            .into_iter()
            .map(|m| symbolize_module(m, &symbols))
            .collect::<Result<Vec<_>, _>>()?;
        packages.insert(
            name.clone(),
            PackageAbi {
                name: name.clone(),
                modules,
            },
        );
        let linkage = f
            .package
            .linkage
            .iter()
            .map(|l| {
                let dep = names.get(&addr(&l.original_id)).cloned().ok_or_else(|| {
                    Error::Closure(format!(
                        "{name} links {} which was not fetched",
                        l.original_id
                    ))
                })?;
                Ok((
                    dep,
                    LinkedVersion {
                        upgraded_id: l.upgraded_id.clone(),
                        version: l.version,
                    },
                ))
            })
            .collect::<Result<_, Error>>()?;
        deployments.insert(
            name,
            PackageDeployment {
                original_id: original.to_hex(),
                published_at: f.package.id.to_hex(),
                version: f.package.version,
                type_origins: f
                    .package
                    .type_origins
                    .iter()
                    .map(|o| {
                        (
                            format!("{}::{}", o.module, o.datatype),
                            o.defining_id.clone(),
                        )
                    })
                    .collect(),
                linkage,
            },
        );
    }

    let chain = source.chain(network)?;
    let system_objects = SYSTEM_OBJECTS
        .iter()
        .filter_map(|(id, name)| {
            chain.system_objects.get(&addr(id)).map(|version| {
                (
                    (*name).to_owned(),
                    SystemObject {
                        id: addr(id).to_hex(),
                        initial_shared_version: *version,
                    },
                )
            })
        })
        .collect();

    Ok(NetworkLock {
        deployment: Deployment {
            lock_version: LOCK_VERSION,
            network: network.to_owned(),
            chain_identifier: chain.chain_identifier,
            checkpoint: chain.checkpoint,
            system_objects,
            packages: deployments,
        },
        packages,
    })
}

/// A package's original id: the self address shared by all its modules.
fn original_id(modules: &[abi::Module], fetched_as: &Address) -> Result<Address, Error> {
    let mut ids = modules.iter().map(|m| m.address.as_str());
    let first = ids
        .next()
        .ok_or_else(|| Error::Package(format!("{fetched_as}: package has no modules")))?;
    if ids.any(|a| a != first) {
        return Err(Error::Package(format!(
            "{fetched_as}: modules disagree on their address"
        )));
    }
    Ok(addr(first))
}

fn symbol(address: &str, symbols: &BTreeMap<String, String>) -> Result<String, Error> {
    symbols.get(address).cloned().ok_or_else(|| {
        Error::Closure(format!(
            "{address} is referenced but not in the dependency closure"
        ))
    })
}

fn symbolize_type(t: &mut Type, symbols: &BTreeMap<String, String>) -> Result<(), Error> {
    match t {
        Type::Vector(inner) | Type::Reference(inner) | Type::MutableReference(inner) => {
            symbolize_type(inner, symbols)
        }
        Type::Datatype(d) => {
            d.address = symbol(&d.address, symbols)?;
            d.type_arguments
                .iter_mut()
                .try_for_each(|a| symbolize_type(a, symbols))
        }
        _ => Ok(()),
    }
}

fn symbolize_module(
    mut m: abi::Module,
    symbols: &BTreeMap<String, String>,
) -> Result<abi::Module, Error> {
    m.address = symbol(&m.address, symbols)?;
    for friend in &mut m.friends {
        friend.address = symbol(&friend.address, symbols)?;
    }
    for d in &mut m.datatypes {
        let fields: Vec<&mut abi::Field> = match &mut d.kind {
            DatatypeKind::Struct(fs) => fs.iter_mut().collect(),
            DatatypeKind::Enum(vs) => vs.iter_mut().flat_map(|v| v.fields.iter_mut()).collect(),
            DatatypeKind::NativeStruct => Vec::new(),
        };
        for f in fields {
            symbolize_type(&mut f.type_, symbols)?;
        }
    }
    for f in &mut m.functions {
        for t in f.parameters.iter_mut().chain(f.returns.iter_mut()) {
            symbolize_type(t, symbols)?;
        }
    }
    for c in &mut m.constants {
        symbolize_type(&mut c.type_, symbols)?;
    }
    Ok(m)
}

/// A difference between two networks' ABIs for the same package (L-NET-1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbiDifference {
    /// Package symbol.
    pub package: String,
    /// Module name.
    pub module: String,
    /// What differs.
    pub detail: String,
}

/// Compares every package present on both `reference` and `other`.
pub fn compare_networks(lock: &Lock, reference: &str, other: &str) -> Vec<AbiDifference> {
    let (Some(a), Some(b)) = (lock.networks.get(reference), lock.networks.get(other)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (name, pa) in &a.packages {
        let Some(pb) = b.packages.get(name) else {
            continue;
        };
        let mb: BTreeMap<_, _> = pb.modules.iter().map(|m| (m.name.as_str(), m)).collect();
        for ma in &pa.modules {
            let diff = |detail: String| AbiDifference {
                package: name.clone(),
                module: ma.name.clone(),
                detail,
            };
            match mb.get(ma.name.as_str()) {
                None => out.push(diff(format!("module missing on {other}"))),
                Some(m) if *m != ma => {
                    let fa: BTreeMap<_, _> =
                        ma.functions.iter().map(|f| (f.name.as_str(), f)).collect();
                    let fb: BTreeMap<_, _> =
                        m.functions.iter().map(|f| (f.name.as_str(), f)).collect();
                    for (f, sig) in &fa {
                        match fb.get(f) {
                            None => out.push(diff(format!("function {f} missing on {other}"))),
                            Some(s) if s != sig => out.push(diff(format!("function {f} differs"))),
                            _ => {}
                        }
                    }
                    for f in fb.keys().filter(|f| !fa.contains_key(*f)) {
                        out.push(diff(format!("function {f} only on {other}")));
                    }
                    if ma.datatypes != m.datatypes {
                        out.push(diff("datatypes differ".to_owned()));
                    }
                }
                _ => {}
            }
        }
    }
    out
}
