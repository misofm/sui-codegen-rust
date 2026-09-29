//! Every package in the fixture corpus (mainnet + testnet: framework, `DeepBook`, all Miso
//! packages and their linked dependencies) reads cleanly.

use std::path::PathBuf;

use movegen_bytecode::{Package, abi::DatatypeKind};

fn corpus() -> Vec<(String, Vec<u8>)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/packages");
    let mut out = Vec::new();
    for network in ["mainnet", "testnet"] {
        for entry in std::fs::read_dir(root.join(network)).expect("run scripts/fetch-fixtures.py") {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "bcs") {
                out.push((
                    format!("{network}/{}", path.file_name().unwrap().to_string_lossy()),
                    std::fs::read(&path).unwrap(),
                ));
            }
        }
    }
    out.sort();
    out
}

// spec: B-PKG-1, B-HDR-1..B-CODE-1, B-IDX-1
#[test]
fn whole_corpus_reads() {
    let corpus = corpus();
    assert!(corpus.len() >= 100, "corpus too small: {}", corpus.len());
    let (mut modules, mut enums, mut functions) = (0, 0, 0);
    for (name, bytes) in &corpus {
        let package = Package::from_bcs(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        for module in package
            .module_abis()
            .unwrap_or_else(|e| panic!("{name}: {e}"))
        {
            modules += 1;
            functions += module.functions.len();
            enums += module
                .datatypes
                .iter()
                .filter(|d| matches!(d.kind, DatatypeKind::Enum(_)))
                .count();
        }
    }
    eprintln!(
        "{} packages, {modules} modules, {functions} functions, {enums} enums",
        corpus.len()
    );
    assert!(enums > 0, "corpus should contain Move enums");
}

// spec: B-DEF-1 — Record Shop `listing::Pricing` variants keep declaration (tag) order.
#[test]
fn enum_variants_in_tag_order() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/packages/mainnet");
    let bytes = std::fs::read(
        root.join("0x375b296ed49b461b2fae471ab6d6a4a5acedfb6fe64e022e8140791babeb7b40.bcs"),
    )
    .unwrap();
    let package = Package::from_bcs(&bytes).unwrap();
    let listing = package
        .module_abis()
        .unwrap()
        .into_iter()
        .find(|m| m.name == "listing")
        .unwrap();
    let variants = |name: &str| -> Vec<String> {
        match &listing
            .datatypes
            .iter()
            .find(|d| d.name == name)
            .unwrap()
            .kind
        {
            DatatypeKind::Enum(vs) => vs.iter().map(|v| v.name.clone()).collect(),
            other => panic!("{name} is {other:?}"),
        }
    };
    assert_eq!(variants("Pricing"), ["Fixed", "Floor"]);
    // Declared Enabled, Disabled — not alphabetical (GraphQL `layout` gets this wrong).
    assert_eq!(variants("State"), ["Enabled", "Disabled"]);
}
