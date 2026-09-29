//! B-ORACLE-1: our reader agrees with Mysten's `move-binary-format` on every module of the corpus,
//! and on accept/reject for systematically corrupted modules.

use std::path::PathBuf;

use move_binary_format::CompiledModule as TheirModule;
use move_binary_format::binary_config::BinaryConfig;
use move_binary_format::file_format::{
    AbilitySet as TheirAbilities, StructFieldInformation, Visibility as TheirVisibility,
};
use move_binary_format::normalized::{self, NoPool};
use movegen_bytecode::{Error, Package};
use movegen_bytecode::abi::{
    Constant, Datatype, DatatypeKind, DatatypeRef, Field, Function, FunctionVisibility, Module,
    ModuleRef, Type, TypeParameter, Variant,
};

/// Sui's loader configuration: versions 6–7, no extraneous bytes, no global storage.
fn sui_config() -> BinaryConfig {
    BinaryConfig::legacy(7, 6, true, true)
}

fn corpus_modules() -> Vec<(String, Vec<u8>)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/packages");
    let mut out = Vec::new();
    for network in ["mainnet", "testnet"] {
        let mut paths: Vec<_> = std::fs::read_dir(root.join(network))
            .expect("run scripts/fetch-fixtures.py")
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "bcs"))
            .collect();
        paths.sort();
        for path in paths {
            let package = Package::from_bcs(&std::fs::read(&path).unwrap()).unwrap();
            for (name, bytes) in package.modules {
                out.push((format!("{network}/{}::{name}", package.id), bytes));
            }
        }
    }
    out
}

fn abilities(set: TheirAbilities) -> Vec<String> {
    use move_binary_format::file_format::Ability;
    [(Ability::Copy, "copy"), (Ability::Drop, "drop"), (Ability::Store, "store"), (Ability::Key, "key")]
        .into_iter()
        .filter(|(a, _)| set.has_ability(*a))
        .map(|(_, n)| n.to_owned())
        .collect()
}

fn ty(t: &normalized::Type<move_core_types::identifier::Identifier>) -> Type {
    use normalized::Type as T;
    match t {
        T::Bool => Type::Bool,
        T::U8 => Type::U8,
        T::U16 => Type::U16,
        T::U32 => Type::U32,
        T::U64 => Type::U64,
        T::U128 => Type::U128,
        T::U256 => Type::U256,
        T::Address => Type::Address,
        T::Signer => Type::Signer,
        T::Vector(inner) => Type::Vector(Box::new(ty(inner))),
        T::Reference(false, inner) => Type::Reference(Box::new(ty(inner))),
        T::Reference(true, inner) => Type::MutableReference(Box::new(ty(inner))),
        T::TypeParameter(i) => Type::TypeParameter(*i),
        T::Datatype(d) => Type::Datatype(Box::new(DatatypeRef {
            address: d.module.address.to_hex_literal_padded(),
            module: d.module.name.to_string(),
            name: d.name.to_string(),
            type_arguments: d.type_arguments.iter().map(ty).collect(),
        })),
    }
}

trait PaddedHex {
    fn to_hex_literal_padded(&self) -> String;
}
impl PaddedHex for move_core_types::account_address::AccountAddress {
    fn to_hex_literal_padded(&self) -> String {
        format!("0x{}", hex::encode(self.into_bytes()))
    }
}

/// Mysten's view of a module, converted into our ABI shape.
fn their_abi(compiled: &TheirModule) -> Module {
    let n = normalized::Module::new(&mut NoPool, compiled, false);
    let fields = |fs: &normalized::Fields<move_core_types::identifier::Identifier>| -> Vec<Field> {
        fs.0.values().map(|f| Field { name: f.name.to_string(), type_: ty(&f.type_) }).collect()
    };
    let type_params = |ps: &[move_binary_format::file_format::DatatypeTyParameter]| {
        ps.iter()
            .map(|p| TypeParameter { constraints: abilities(p.constraints), phantom: p.is_phantom })
            .collect()
    };
    let mut datatypes = Vec::new();
    for (def, s) in compiled.struct_defs().iter().zip(n.structs.values()) {
        let kind = match def.field_information {
            StructFieldInformation::Native => DatatypeKind::NativeStruct,
            StructFieldInformation::Declared(_) => DatatypeKind::Struct(fields(&s.fields)),
        };
        datatypes.push(Datatype {
            name: s.name.to_string(),
            abilities: abilities(s.abilities),
            type_parameters: type_params(&s.type_parameters),
            kind,
        });
    }
    for e in n.enums.values() {
        datatypes.push(Datatype {
            name: e.name.to_string(),
            abilities: abilities(e.abilities),
            type_parameters: type_params(&e.type_parameters),
            kind: DatatypeKind::Enum(
                e.variants.values().map(|v| Variant { name: v.name.to_string(), fields: fields(&v.fields) }).collect(),
            ),
        });
    }
    let functions = compiled
        .function_defs()
        .iter()
        .zip(n.functions.values())
        .map(|(def, f)| Function {
            name: f.name.to_string(),
            visibility: match f.visibility {
                TheirVisibility::Private => FunctionVisibility::Private,
                TheirVisibility::Public => FunctionVisibility::Public,
                TheirVisibility::Friend => FunctionVisibility::Friend,
            },
            entry: f.is_entry,
            native: def.code.is_none(),
            type_parameters: f.type_parameters.iter().map(|a| abilities(*a)).collect(),
            parameters: f.parameters.iter().map(|t| ty(t)).collect(),
            returns: f.return_.iter().map(|t| ty(t)).collect(),
        })
        .collect();
    Module {
        address: n.id.address.to_hex_literal_padded(),
        name: n.id.name.to_string(),
        bytecode_version: n.file_format_version,
        friends: n
            .friends
            .iter()
            .map(|m| ModuleRef { address: m.address.to_hex_literal_padded(), name: m.name.to_string() })
            .collect(),
        datatypes,
        functions,
        constants: n
            .constants
            .iter()
            .map(|c| Constant { type_: ty(&c.type_), data: hex::encode(&c.data) })
            .collect(),
    }
}

// spec: B-ORACLE-1
#[test]
fn every_module_matches_move_binary_format() {
    let modules = corpus_modules();
    assert!(modules.len() > 300, "corpus has {} modules", modules.len());
    for (name, bytes) in &modules {
        let theirs = TheirModule::deserialize_with_config(bytes, &sui_config())
            .unwrap_or_else(|e| panic!("{name}: Mysten rejected a corpus module: {e:?}"));
        let ours = Module::from_bytes(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(ours, their_abi(&theirs), "{name}");
    }
    eprintln!("{} modules agree", modules.len());
}

/// Deterministic corruptions of real modules: flip each byte position (strided) and truncate.
// spec: B-ORACLE-1 (rejection agreement), B-ORACLE-2
#[test]
fn corruption_agreement() {
    corruption_sweep(7, 64);
}

/// The same property, dense: every module, ~256 positions each. `cargo test --release -- --ignored`.
#[test]
#[ignore = "slow; run before releases"]
fn corruption_agreement_dense() {
    corruption_sweep(1, 256);
}

fn corruption_sweep(module_step: usize, positions: usize) {
    let modules = corpus_modules();
    let (mut both_reject, mut both_accept, mut we_are_stricter, mut they_are_stricter) = (0, 0, 0, 0);
    let mut stricter_examples = Vec::new();
    for (name, bytes) in modules.iter().step_by(module_step) {
        let original = Module::from_bytes(bytes).unwrap();
        let stride = (bytes.len() / positions).max(1);
        let mut mutants: Vec<(String, Vec<u8>)> = Vec::new();
        for pos in (0..bytes.len()).step_by(stride) {
            for delta in [0x01u8, 0x80, 0xff] {
                let mut m = bytes.clone();
                m[pos] ^= delta;
                mutants.push((format!("{name} @{pos} ^{delta:#x}"), m));
            }
        }
        for cut in [1, 7, bytes.len() / 2] {
            if cut < bytes.len() {
                mutants.push((format!("{name} truncated -{cut}"), bytes[..bytes.len() - cut].to_vec()));
            }
        }
        mutants.push((format!("{name} + trailing byte"), [bytes.as_slice(), &[0]].concat()));
        for (label, m) in mutants {
            let theirs = TheirModule::deserialize_with_config(&m, &sui_config());
            let ours = Module::from_bytes(&m);
            match (&ours, &theirs) {
                (Err(_), Err(_)) => both_reject += 1,
                (Ok(o), Ok(t)) => {
                    both_accept += 1;
                    assert_eq!(o, &their_abi(t), "{label}: both accepted but ABIs differ");
                }
                // We reject something Mysten's deserializer accepts: allowed only for the
                // verifier-level structural rules B-ABI-1..3; anything else is a reader bug.
                (Err(e), Ok(_)) => match e {
                    Error::Duplicate { .. } | Error::ForeignDefinition { .. } | Error::IllFormedType(_) => {
                        we_are_stricter += 1;
                    }
                    other => panic!("{label}: we reject ({other}) but Mysten accepts"),
                },
                // We accept something Mysten rejects: allowed only when the corruption is
                // invisible to the ABI (it hit code bodies or code-only tables) — B-ORACLE-2.
                (Ok(o), Err(e)) => {
                    assert_eq!(o, &original, "{label}: Mysten rejects ({:?}) and our ABI changed", e.major_status());
                    they_are_stricter += 1;
                    if stricter_examples.len() < 3 {
                        stricter_examples.push(format!("{:?}", e.major_status()));
                    }
                }
            }
        }
    }
    eprintln!(
        "mutants: both reject {both_reject}, both accept {both_accept}, we stricter (B-ABI) {we_are_stricter}, Mysten stricter {they_are_stricter}, examples: {stricter_examples:#?}"
    );
}
