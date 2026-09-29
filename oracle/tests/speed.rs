//! Rough speed comparison on the corpus (not a CI gate): `cargo test --release --test speed -- --nocapture --ignored`.
use std::{path::PathBuf, time::Instant};

use move_binary_format::CompiledModule as TheirModule;
use move_binary_format::binary_config::BinaryConfig;
use move_binary_format::normalized::{self, NoPool};
use movegen_bytecode::{Package, abi::Module};

fn modules() -> Vec<Vec<u8>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/packages");
    let mut out = Vec::new();
    for network in ["mainnet", "testnet"] {
        for e in std::fs::read_dir(root.join(network)).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "bcs") {
                out.extend(Package::from_bcs(&std::fs::read(&p).unwrap()).unwrap().modules.into_values());
            }
        }
    }
    out
}

fn time(label: &str, iters: u32, mut f: impl FnMut()) {
    f(); // warm up
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    let per = start.elapsed() / iters;
    println!("{label:<52} {:>9.2?} per corpus pass", per);
}

#[test]
#[ignore = "benchmark"]
fn speed() {
    let modules = modules();
    let bytes: usize = modules.iter().map(Vec::len).sum();
    println!("{} modules, {} KiB", modules.len(), bytes / 1024);
    let config = BinaryConfig::legacy(7, 6, true, true);
    time("ours: deserialize + bounds check", 50, || {
        for m in &modules { std::hint::black_box(movegen_bytecode::deserialize(m).unwrap()); }
    });
    time("mysten: deserialize + bounds check", 50, || {
        for m in &modules { std::hint::black_box(TheirModule::deserialize_with_config(m, &config).unwrap()); }
    });
    time("ours: + resolved ABI (names, types)", 50, || {
        for m in &modules { std::hint::black_box(Module::from_bytes(m).unwrap()); }
    });
    time("mysten: + normalized module (no code)", 50, || {
        for m in &modules {
            let c = TheirModule::deserialize_with_config(m, &config).unwrap();
            std::hint::black_box(normalized::Module::new(&mut NoPool, &c, false));
        }
    });
}
