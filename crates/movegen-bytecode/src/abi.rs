//! A normalized, name-resolved view of a module's ABI.
//!
//! This is the shape the lock file stores and the oracle compares against Mysten's
//! `move-binary-format` (B-ORACLE-1). Every index is resolved and bounds-checked (B-IDX-1).

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::model::{AbilitySet, CompiledModule, FieldDefinition, Token, Visibility};
use crate::sig::{Node, NodeReader};

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
    set.names().map(str::to_owned).collect()
}

impl Module {
    /// Resolves a compiled module (B-IDX-1, B-ABI-1..3).
    pub fn from_compiled(m: &CompiledModule<'_>) -> Result<Self, Error> {
        let (address, name) = m.self_id()?;
        // Each pool address is hex-encoded once, not once per type reference.
        let hex: Vec<String> = m
            .addresses
            .chunks_exact(32)
            .map(|a| {
                let mut s = String::with_capacity(66);
                s.push_str("0x");
                s.push_str(&hex::encode(a));
                s
            })
            .collect();
        let cx = Cx { m, hex: &hex };
        let fields = |fs: &[FieldDefinition<'_>], arity: usize| -> Result<Vec<Field>, Error> {
            fs.iter()
                .map(|f| {
                    Ok(Field {
                        name: m.identifier(f.name)?.to_owned(),
                        type_: resolve(&cx, f.type_, arity)?,
                    })
                })
                .collect()
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
                type_parameters: m
                    .datatype_type_params(h)
                    .iter()
                    .map(|p| TypeParameter {
                        constraints: abilities(p.constraints),
                        phantom: p.is_phantom,
                    })
                    .collect(),
                kind,
            })
        };
        let arity_of =
            |handle: u16| -> Result<usize, Error> { Ok(m.datatype_handle(handle)?.arity()) };

        let mut datatypes = Vec::with_capacity(m.struct_defs.len() + m.enum_defs.len());
        for def in &m.struct_defs {
            let kind = match m.struct_fields(def) {
                Some(fs) => DatatypeKind::Struct(fields(fs, arity_of(def.handle)?)?),
                None => DatatypeKind::NativeStruct,
            };
            datatypes.push(datatype(def.handle, kind)?);
        }
        for def in &m.enum_defs {
            let arity = arity_of(def.handle)?;
            let variants = m
                .enum_variants(def)
                .iter()
                .map(|v| {
                    Ok(Variant {
                        name: m.identifier(v.name)?.to_owned(),
                        fields: fields(m.variant_fields(v), arity)?,
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
                let arity = h.arity();
                let types = |sig: u16| -> Result<Vec<Type>, Error> {
                    m.signature(sig)?
                        .tokens()
                        .map(|t| resolve(&cx, t, arity))
                        .collect()
                };
                Ok(Function {
                    name: m.identifier(h.name)?.to_owned(),
                    visibility: match def.visibility {
                        Visibility::Private => FunctionVisibility::Private,
                        Visibility::Public => FunctionVisibility::Public,
                        Visibility::Friend => FunctionVisibility::Friend,
                    },
                    entry: def.is_entry,
                    native: def.is_native,
                    type_parameters: m
                        .function_type_params(h)
                        .iter()
                        .map(|a| abilities(*a))
                        .collect(),
                    parameters: types(h.parameters)?,
                    returns: types(h.return_)?,
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
                    type_: resolve(&cx, c.type_, 0)?,
                    data: hex::encode(c.data),
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

/// B-ABI-1: names are unique. Lists are small, so a sort beats hashing.
fn unique<'a>(what: &'static str, names: impl Iterator<Item = &'a str>) -> Result<(), Error> {
    let mut names: Vec<&str> = names.collect();
    names.sort_unstable();
    match names.windows(2).find(|w| w[0] == w[1]) {
        Some(w) => Err(Error::Duplicate {
            what,
            name: w[0].to_owned(),
        }),
        None => Ok(()),
    }
}

struct Cx<'k, 'a> {
    m: &'k CompiledModule<'a>,
    hex: &'k [String],
}

/// Decodes a token in a context with `arity` type parameters (B-ABI-3).
fn resolve(cx: &Cx<'_, '_>, token: Token<'_>, arity: usize) -> Result<Type, Error> {
    fn go(cx: &Cx<'_, '_>, r: &mut NodeReader<'_>, arity: usize) -> Result<Type, Error> {
        let m = cx.m;
        let datatype = |handle: u16, args: Vec<Type>| -> Result<Type, Error> {
            let h = m.datatype_handle(handle)?;
            let mh = m.module_handle(h.module)?;
            let address = cx
                .hex
                .get(usize::from(mh.address))
                .ok_or(Error::OutOfBounds {
                    what: "address",
                    index: usize::from(mh.address),
                    len: cx.hex.len(),
                })?;
            let module = m.identifier(mh.name)?;
            Ok(Type::Datatype(Box::new(DatatypeRef {
                address: address.clone(),
                module: module.to_owned(),
                name: m.identifier(h.name)?.to_owned(),
                type_arguments: args,
            })))
        };
        Ok(match r.next()? {
            Node::Bool => Type::Bool,
            Node::U8 => Type::U8,
            Node::U16 => Type::U16,
            Node::U32 => Type::U32,
            Node::U64 => Type::U64,
            Node::U128 => Type::U128,
            Node::U256 => Type::U256,
            Node::Address => Type::Address,
            Node::Signer => Type::Signer,
            Node::Vector => Type::Vector(Box::new(go(cx, r, arity)?)),
            Node::Reference => Type::Reference(Box::new(go(cx, r, arity)?)),
            Node::MutableReference => Type::MutableReference(Box::new(go(cx, r, arity)?)),
            Node::Datatype(h) => datatype(h, Vec::new())?,
            Node::Instantiation(h, n) => {
                let args = (0..n).map(|_| go(cx, r, arity)).collect::<Result<_, _>>()?;
                datatype(h, args)?
            }
            Node::TypeParameter(i) if usize::from(i) < arity => Type::TypeParameter(i),
            Node::TypeParameter(i) => {
                return Err(Error::IllFormedType(format!(
                    "type parameter {i} out of range (arity {arity})"
                )));
            }
        })
    }
    go(cx, &mut NodeReader::new(token), arity)
}
