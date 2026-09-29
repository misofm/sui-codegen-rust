//! `refresh` over the fixture corpus: closure, symbolization, deployments, determinism, and
//! cross-network comparison (docs/SPEC.md §L).

mod common;

use common::{Fixtures, miso_config};
use movegen::lock::Lock;
use movegen::{compare_networks, refresh};
use movegen_bytecode::abi::{DatatypeKind, Type};

fn assert_symbolized(t: &Type) {
    match t {
        Type::Vector(i) | Type::Reference(i) | Type::MutableReference(i) => assert_symbolized(i),
        Type::Datatype(d) => {
            assert!(d.address.starts_with('@'), "unsymbolized {}", d.address);
            d.type_arguments.iter().for_each(assert_symbolized);
        }
        _ => {}
    }
}

// spec: L-CLO-1, L-SYM-1, L-SYM-2
#[test]
fn closure_and_symbolization() {
    let config = miso_config();
    let lock = refresh(&config, &Fixtures).unwrap();
    for (network, part) in &lock.networks {
        let names: Vec<_> = part.packages.keys().cloned().collect();
        assert!(
            names.contains(&"std".to_owned()) && names.contains(&"sui".to_owned()),
            "{network}: {names:?}"
        );
        assert!(
            names.contains(&"record_shop".to_owned()),
            "{network}: {names:?}"
        );
        for abi in part.packages.values() {
            for m in &abi.modules {
                assert_eq!(m.address, format!("@{}", abi.name));
                for d in &m.datatypes {
                    let fields: Vec<_> = match &d.kind {
                        DatatypeKind::Struct(fs) => fs.iter().collect(),
                        DatatypeKind::Enum(vs) => vs.iter().flat_map(|v| &v.fields).collect(),
                        DatatypeKind::NativeStruct => Vec::new(),
                    };
                    for f in &fields {
                        assert_symbolized(&f.type_);
                    }
                }
                m.functions
                    .iter()
                    .flat_map(|f| f.parameters.iter().chain(&f.returns))
                    .for_each(assert_symbolized);
            }
        }
        // Every linked dependency is itself in the lock.
        for (name, dep) in &part.deployment.packages {
            for linked in dep.linkage.keys() {
                assert!(
                    part.packages.contains_key(linked),
                    "{network}: {name} links missing {linked}"
                );
            }
        }
    }
}

// spec: L-DEP-1, L-DEP-2
#[test]
fn deployments() {
    let lock = refresh(&miso_config(), &Fixtures).unwrap();
    let main = &lock.networks["mainnet"].deployment;
    assert_eq!(
        main.chain_identifier,
        "4btiuiMPvEENsttpZC7CZ53DruC3MAgfznDbASZ7DR6S"
    );
    assert_eq!(main.system_objects["clock"].initial_shared_version, 1);
    assert_eq!(
        main.system_objects["random"].initial_shared_version,
        326_168_368
    );
    assert_ne!(
        main.system_objects["random"].initial_shared_version,
        lock.networks["testnet"].deployment.system_objects["random"].initial_shared_version,
        "system object versions are per-network data"
    );
    let shop = &main.packages["record_shop"];
    assert_eq!(
        shop.published_at,
        "0x375b296ed49b461b2fae471ab6d6a4a5acedfb6fe64e022e8140791babeb7b40"
    );
    assert_eq!(shop.type_origins["listing::Listing"], shop.original_id);
    assert!(shop.linkage.contains_key("record"));
}

// spec: L-DET-1
#[test]
fn deterministic_and_round_trips() {
    let config = miso_config();
    let a = refresh(&config, &Fixtures).unwrap();
    let b = refresh(&config, &Fixtures).unwrap();
    assert_eq!(a.files(), b.files());
    let dir = std::env::temp_dir().join(format!("movegen-lock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let changed = a.write(&dir).unwrap();
    assert_eq!(changed.len(), a.files().len());
    assert!(
        a.write(&dir).unwrap().is_empty(),
        "rewriting an identical lock changes nothing"
    );
    assert_eq!(Lock::read(&dir).unwrap(), a);
    std::fs::remove_dir_all(&dir).unwrap();
}

// spec: L-NET-1 — the current Record Shop packages have the same ABI on both networks.
#[test]
fn record_sales_abi_matches_across_networks() {
    let lock = refresh(&miso_config(), &Fixtures).unwrap();
    let diffs: Vec<_> = compare_networks(&lock, "mainnet", "testnet")
        .into_iter()
        .filter(|d| d.package.starts_with("record"))
        .collect();
    assert!(diffs.is_empty(), "{diffs:#?}");
}
