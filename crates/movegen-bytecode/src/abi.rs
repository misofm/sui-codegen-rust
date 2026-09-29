//! A normalized, name-resolved view of a module's ABI.
//!
//! This is the shape the lock file stores and the oracle compares against Mysten's
//! `move-binary-format` (B-ORACLE-1). Every index is resolved and bounds-checked (B-IDX-1).

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::model::{AbilitySet, CompiledModule, SignatureToken, Visibility};

/// A Move type as it appears in a signature.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Type {
    /// `bool`
    Bool,
    /// `u8`
    U8,
    /// `u16`
    U16,
    /// `u32`
    U32,
    /// `u64`
    U64,
    /// `u128`
    U128,
    /// `u256`
    U256,
    /// `address`
    Address,
    /// `signer`
    Signer,
    /// `vector<T>`
    Vector(Box<Type>),
    /// `&T`
    Reference(Box<Type>),
    /// `&mut T`
    MutableReference(Box<Type>),
    /// A datatype, by the address in the referencing module's bytecode (an original id).
    Datatype(Box<DatatypeRef>),
    /// A type parameter of the enclosing datatype or function.
    TypeParameter(u16),
}

/// A reference to a datatype.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DatatypeRef {
    /// `0x` + 64 hex digits.
    pub address: String,
    /// Module name.
    pub module: String,
    /// Datatype name.
    pub name: String,
    /// Type arguments.
    pub type_arguments: Vec<Type>,
}

/// A datatype type parameter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeParameter {
    /// Ability constraints (`copy`, `drop`, `store`, `key`).
    pub constraints: Vec<String>,
    /// Phantom.
    pub phantom: bool,
}

/// A field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    /// Name (`pos0`, `pos1`, … for positional fields; `dummy_field` for empty structs).
    pub name: String,
    /// Type.
    #[serde(rename = "type")]
    pub type_: Type,
}

/// An enum variant, in tag order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Variant {
    /// Name.
    pub name: String,
    /// Fields.
    pub fields: Vec<Field>,
}

/// What kind of datatype.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatatypeKind {
    /// A struct with declared fields.
    Struct(Vec<Field>),
    /// A native struct (no fields visible).
    NativeStruct,
    /// An enum; variants in tag order.
    Enum(Vec<Variant>),
}

/// A datatype defined in the module.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Datatype {
    /// Name.
    pub name: String,
    /// Declared abilities.
    pub abilities: Vec<String>,
    /// Type parameters.
    pub type_parameters: Vec<TypeParameter>,
    /// Struct, native struct, or enum.
    pub kind: DatatypeKind,
}

/// Function visibility.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FunctionVisibility {
    /// Private.
    Private,
    /// Public.
    Public,
    /// `public(package)`.
    Friend,
}

/// A function defined in the module.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Function {
    /// Name.
    pub name: String,
    /// Visibility.
    pub visibility: FunctionVisibility,
    /// `entry`.
    pub entry: bool,
    /// `native`.
    pub native: bool,
    /// Type parameter constraints.
    pub type_parameters: Vec<Vec<String>>,
    /// Parameters (unnamed on chain).
    pub parameters: Vec<Type>,
    /// Return types.
    pub returns: Vec<Type>,
}

/// A constant pool entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Constant {
    /// Type.
    #[serde(rename = "type")]
    pub type_: Type,
    /// BCS value, hex.
    pub data: String,
}

/// A module's `address::name`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ModuleRef {
    /// `0x` + 64 hex digits.
    pub address: String,
    /// Name.
    pub name: String,
}

/// A module's normalized ABI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Module {
    /// The module's self address (the package's original id).
    pub address: String,
    /// Name.
    pub name: String,
    /// Bytecode version.
    pub bytecode_version: u32,
    /// Friend modules.
    pub friends: Vec<ModuleRef>,
    /// Structs (definition order) then enums (definition order).
    pub datatypes: Vec<Datatype>,
    /// Functions, definition order.
    pub functions: Vec<Function>,
    /// Constant pool, pool order.
    pub constants: Vec<Constant>,
}

fn abilities(set: AbilitySet) -> Vec<String> {
    set.names().into_iter().map(str::to_owned).collect()
}

impl Module {
    /// Resolves a compiled module (B-IDX-1).
    pub fn from_compiled(m: &CompiledModule) -> Result<Self, Error> {
        let (address, name) = m.self_id()?;
        let ty = |t: &SignatureToken, arity: usize| resolve_type(m, t, arity);
        let fields =
            |fs: &[crate::model::FieldDefinition], arity: usize| -> Result<Vec<Field>, Error> {
                fs.iter()
                    .map(|f| {
                        Ok(Field {
                            name: m.identifier(f.name)?.to_owned(),
                            type_: ty(&f.type_, arity)?,
                        })
                    })
                    .collect()
            };
        let arity_of = |handle: u16| -> Result<usize, Error> {
            Ok(m.datatype_handle(handle)?.type_parameters.len())
        };
        let datatype = |handle: u16, kind: DatatypeKind| -> Result<Datatype, Error> {
            let h = m.datatype_handle(handle)?;
            if h.module != m.self_handle {
                return Err(Error::ForeignDefinition {
                    what: "datatype",
                    name: m.identifier(h.name)?.to_owned(),
                });
            }
            Ok(Datatype {
                name: m.identifier(h.name)?.to_owned(),
                abilities: abilities(h.abilities),
                type_parameters: h
                    .type_parameters
                    .iter()
                    .map(|p| TypeParameter {
                        constraints: abilities(p.constraints),
                        phantom: p.is_phantom,
                    })
                    .collect(),
                kind,
            })
        };

        let mut datatypes = Vec::new();
        for def in &m.struct_defs {
            let kind = match &def.fields {
                Some(fs) => DatatypeKind::Struct(fields(fs, arity_of(def.handle)?)?),
                None => DatatypeKind::NativeStruct,
            };
            datatypes.push(datatype(def.handle, kind)?);
        }
        for def in &m.enum_defs {
            let variants = def
                .variants
                .iter()
                .map(|v| {
                    let arity = arity_of(def.handle)?;
                    Ok(Variant {
                        name: m.identifier(v.name)?.to_owned(),
                        fields: fields(&v.fields, arity)?,
                    })
                })
                .collect::<Result<_, Error>>()?;
            datatypes.push(datatype(def.handle, DatatypeKind::Enum(variants))?);
        }

        let functions: Vec<Function> = m
            .function_defs
            .iter()
            .map(|def| {
                let h = m.function_handle(def.handle)?;
                if h.module != m.self_handle {
                    return Err(Error::ForeignDefinition {
                        what: "function",
                        name: m.identifier(h.name)?.to_owned(),
                    });
                }
                let arity = h.type_parameters.len();
                Ok(Function {
                    name: m.identifier(h.name)?.to_owned(),
                    visibility: match def.visibility {
                        Visibility::Private => FunctionVisibility::Private,
                        Visibility::Public => FunctionVisibility::Public,
                        Visibility::Friend => FunctionVisibility::Friend,
                    },
                    entry: def.is_entry,
                    native: def.is_native,
                    type_parameters: h.type_parameters.iter().map(|a| abilities(*a)).collect(),
                    parameters: m
                        .signature(h.parameters)?
                        .iter()
                        .map(|t| ty(t, arity))
                        .collect::<Result<_, _>>()?,
                    returns: m
                        .signature(h.return_)?
                        .iter()
                        .map(|t| ty(t, arity))
                        .collect::<Result<_, _>>()?,
                })
            })
            .collect::<Result<_, Error>>()?;

        let friends = m
            .friends
            .iter()
            .map(|h| {
                let (address, name) = m.module_id(h)?;
                Ok(ModuleRef {
                    address: address.to_hex(),
                    name: name.to_owned(),
                })
            })
            .collect::<Result<_, Error>>()?;

        let constants = m
            .constants
            .iter()
            .map(|c| {
                Ok(Constant {
                    type_: ty(&c.type_, 0)?,
                    data: hex::encode(&c.data),
                })
            })
            .collect::<Result<_, Error>>()?;

        unique(
            "datatype",
            datatypes.iter().map(|d: &Datatype| d.name.as_str()),
        )?;
        for d in &datatypes {
            match &d.kind {
                DatatypeKind::Struct(fs) => unique("field", fs.iter().map(|f| f.name.as_str()))?,
                DatatypeKind::Enum(vs) => {
                    unique("variant", vs.iter().map(|v| v.name.as_str()))?;
                    for v in vs {
                        unique("field", v.fields.iter().map(|f| f.name.as_str()))?;
                    }
                }
                DatatypeKind::NativeStruct => {}
            }
        }
        unique(
            "function",
            functions.iter().map(|f: &Function| f.name.as_str()),
        )?;

        Ok(Module {
            address: address.to_hex(),
            name: name.to_owned(),
            bytecode_version: m.version,
            friends,
            datatypes,
            functions,
            constants,
        })
    }

    /// Reads and resolves a module from bytecode.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        Self::from_compiled(&crate::deserialize(bytes)?)
    }
}

/// B-ABI-1: definition names are unique.
fn unique<'a>(what: &'static str, names: impl Iterator<Item = &'a str>) -> Result<(), Error> {
    let mut seen = std::collections::HashSet::new();
    for name in names {
        if !seen.insert(name) {
            return Err(Error::Duplicate {
                what,
                name: name.to_owned(),
            });
        }
    }
    Ok(())
}

/// Resolves a signature token in a context with `arity` type parameters (B-ABI-3).
fn resolve_type(m: &CompiledModule, t: &SignatureToken, arity: usize) -> Result<Type, Error> {
    let datatype = |handle: u16, args: &[SignatureToken]| -> Result<Type, Error> {
        let h = m.datatype_handle(handle)?;
        if args.len() != h.type_parameters.len() {
            return Err(Error::IllFormedType(format!(
                "{} expects {} type arguments, got {}",
                m.identifier(h.name)?,
                h.type_parameters.len(),
                args.len()
            )));
        }
        let (address, module) = m.module_id(m.module_handle(h.module)?)?;
        Ok(Type::Datatype(Box::new(DatatypeRef {
            address: address.to_hex(),
            module: module.to_owned(),
            name: m.identifier(h.name)?.to_owned(),
            type_arguments: args
                .iter()
                .map(|a| resolve_type(m, a, arity))
                .collect::<Result<_, _>>()?,
        })))
    };
    Ok(match t {
        SignatureToken::Bool => Type::Bool,
        SignatureToken::U8 => Type::U8,
        SignatureToken::U16 => Type::U16,
        SignatureToken::U32 => Type::U32,
        SignatureToken::U64 => Type::U64,
        SignatureToken::U128 => Type::U128,
        SignatureToken::U256 => Type::U256,
        SignatureToken::Address => Type::Address,
        SignatureToken::Signer => Type::Signer,
        SignatureToken::Vector(inner) => Type::Vector(Box::new(resolve_type(m, inner, arity)?)),
        SignatureToken::Reference(inner) => {
            Type::Reference(Box::new(resolve_type(m, inner, arity)?))
        }
        SignatureToken::MutableReference(inner) => {
            Type::MutableReference(Box::new(resolve_type(m, inner, arity)?))
        }
        SignatureToken::Datatype(h) => datatype(*h, &[])?,
        SignatureToken::DatatypeInstantiation(h, args) => datatype(*h, args)?,
        SignatureToken::TypeParameter(i) if usize::from(*i) < arity => Type::TypeParameter(*i),
        SignatureToken::TypeParameter(i) => {
            return Err(Error::IllFormedType(format!(
                "type parameter {i} out of range (arity {arity})"
            )));
        }
    })
}
