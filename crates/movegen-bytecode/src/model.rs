//! The raw tables of a compiled module that describe its ABI.
//!
//! Mirrors `move-binary-format`'s `CompiledModule` for the tables we keep. Indices are raw `u16`s
//! into the module's pools; resolve them through [`CompiledModule`]'s accessors, which
//! bounds-check (B-IDX-1).

use crate::error::Error;

/// A 32-byte account address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Address(pub [u8; 32]);

impl Address {
    /// `0x` + 64 lowercase hex digits.
    pub fn to_hex(&self) -> String {
        format!("0x{}", hex::encode(self.0))
    }

    /// Parses `0x`-prefixed or bare hex, 1–64 digits.
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.strip_prefix("0x").unwrap_or(s);
        if s.is_empty() || s.len() > 64 {
            return None;
        }
        let padded = format!("{s:0>64}");
        let bytes = hex::decode(padded).ok()?;
        Some(Address(bytes.try_into().ok()?))
    }
}

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// A set of Move abilities (B-ABIL-1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AbilitySet(pub u8);

impl AbilitySet {
    /// `copy`
    pub const COPY: u8 = 0x1;
    /// `drop`
    pub const DROP: u8 = 0x2;
    /// `store`
    pub const STORE: u8 = 0x4;
    /// `key`
    pub const KEY: u8 = 0x8;

    /// Whether every bit in `ability` is set.
    pub fn has(self, ability: u8) -> bool {
        self.0 & ability == ability
    }

    /// Ability names in declaration order (`copy`, `drop`, `store`, `key`).
    pub fn names(self) -> Vec<&'static str> {
        [
            (Self::COPY, "copy"),
            (Self::DROP, "drop"),
            (Self::STORE, "store"),
            (Self::KEY, "key"),
        ]
        .into_iter()
        .filter(|(bit, _)| self.has(*bit))
        .map(|(_, name)| name)
        .collect()
    }
}

/// Function visibility (B-FN-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Visibility {
    /// Only the defining module.
    Private,
    /// Anyone.
    Public,
    /// `public(package)`: modules in the friend list.
    Friend,
}

/// A module handle: `address::name`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleHandle {
    /// Index into the address pool.
    pub address: u16,
    /// Index into the identifier pool.
    pub name: u16,
}

/// A datatype type parameter (B-TP-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DatatypeTypeParameter {
    /// Ability constraints.
    pub constraints: AbilitySet,
    /// Whether the parameter is phantom.
    pub is_phantom: bool,
}

/// A datatype handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatatypeHandle {
    /// Defining module handle.
    pub module: u16,
    /// Name identifier.
    pub name: u16,
    /// Declared abilities.
    pub abilities: AbilitySet,
    /// Type parameters.
    pub type_parameters: Vec<DatatypeTypeParameter>,
}

/// A function handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionHandle {
    /// Defining module handle.
    pub module: u16,
    /// Name identifier.
    pub name: u16,
    /// Parameter signature index.
    pub parameters: u16,
    /// Return signature index.
    pub return_: u16,
    /// Type parameter constraints.
    pub type_parameters: Vec<AbilitySet>,
}

/// A signature token (B-SIG-1).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SignatureToken {
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
    Vector(Box<SignatureToken>),
    /// `&T`
    Reference(Box<SignatureToken>),
    /// `&mut T`
    MutableReference(Box<SignatureToken>),
    /// A datatype with no type arguments (datatype handle index).
    Datatype(u16),
    /// A datatype instantiation (datatype handle index, type arguments).
    DatatypeInstantiation(u16, Vec<SignatureToken>),
    /// A type parameter (index).
    TypeParameter(u16),
}

/// A constant pool entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Constant {
    /// Constant type.
    pub type_: SignatureToken,
    /// BCS-encoded value.
    pub data: Vec<u8>,
}

/// A field definition (name index, type).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldDefinition {
    /// Name identifier.
    pub name: u16,
    /// Field type.
    pub type_: SignatureToken,
}

/// A struct definition (B-DEF-1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructDefinition {
    /// Datatype handle.
    pub handle: u16,
    /// Fields; `None` for native structs.
    pub fields: Option<Vec<FieldDefinition>>,
}

/// An enum variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantDefinition {
    /// Name identifier.
    pub name: u16,
    /// Fields.
    pub fields: Vec<FieldDefinition>,
}

/// An enum definition (B-DEF-1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumDefinition {
    /// Datatype handle.
    pub handle: u16,
    /// Variants in declaration (tag) order.
    pub variants: Vec<VariantDefinition>,
}

/// A function definition (B-FN-1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionDefinition {
    /// Function handle.
    pub handle: u16,
    /// Visibility.
    pub visibility: Visibility,
    /// `entry`.
    pub is_entry: bool,
    /// `native` (no code).
    pub is_native: bool,
    /// The code unit's index operands (kept only for bounds checking); `None` if native.
    pub code: Option<CodeUnit>,
}

/// The operands of a code unit that index into module tables (B-CODE-1). Instruction semantics
/// are not modelled; only what the bounds checker needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeUnit {
    /// Locals signature index.
    pub locals: u16,
    /// One entry per instruction.
    pub code: Vec<Operand>,
    /// Variant jump tables: `(head enum definition, branch offsets)`.
    pub jump_tables: Vec<(u16, Vec<u16>)>,
}

/// An instruction's table-indexing operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operand {
    /// No indexing operand.
    None,
    /// Branch target (code offset).
    Branch(u16),
    /// Constant pool index.
    Constant(u16),
    /// Local index.
    Local(u8),
    /// Field handle index.
    FieldHandle(u16),
    /// Field instantiation index.
    FieldInstantiation(u16),
    /// Function handle index.
    FunctionHandle(u16),
    /// Function instantiation index.
    FunctionInstantiation(u16),
    /// Struct definition index.
    StructDefinition(u16),
    /// Struct instantiation index.
    StructInstantiation(u16),
    /// Vector instruction element signature index.
    VectorSignature(u16),
    /// Variant handle index.
    VariantHandle(u16),
    /// Variant instantiation handle index.
    VariantInstantiation(u16),
    /// Jump table index.
    JumpTable(u16),
}

/// Instantiation-style tables: `(definition or handle, type-argument signature)`.
pub type Instantiation = (u16, u16);

/// The ABI-relevant tables of a compiled module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledModule {
    /// Bytecode version (6 or 7).
    pub version: u32,
    /// The module's own handle.
    pub self_handle: u16,
    /// Module handles.
    pub module_handles: Vec<ModuleHandle>,
    /// Datatype handles.
    pub datatype_handles: Vec<DatatypeHandle>,
    /// Function handles.
    pub function_handles: Vec<FunctionHandle>,
    /// Signature pool.
    pub signatures: Vec<Vec<SignatureToken>>,
    /// Identifier pool.
    pub identifiers: Vec<String>,
    /// Address pool.
    pub addresses: Vec<Address>,
    /// Constant pool.
    pub constants: Vec<Constant>,
    /// Struct definitions, in definition order.
    pub struct_defs: Vec<StructDefinition>,
    /// Enum definitions, in definition order.
    pub enum_defs: Vec<EnumDefinition>,
    /// Function definitions, in definition order.
    pub function_defs: Vec<FunctionDefinition>,
    /// Friend modules.
    pub friends: Vec<ModuleHandle>,
    /// Function instantiations `(function handle, signature)`.
    pub function_instantiations: Vec<Instantiation>,
    /// Struct instantiations `(struct definition, signature)`.
    pub struct_instantiations: Vec<Instantiation>,
    /// Enum instantiations `(enum definition, signature)`.
    pub enum_instantiations: Vec<Instantiation>,
    /// Field handles `(struct definition, field offset)`.
    pub field_handles: Vec<(u16, u16)>,
    /// Field instantiations `(field handle, signature)`.
    pub field_instantiations: Vec<Instantiation>,
    /// Variant handles `(enum definition, variant tag)`.
    pub variant_handles: Vec<(u16, u16)>,
    /// Variant instantiation handles `(enum instantiation, variant tag)`.
    pub variant_instantiation_handles: Vec<(u16, u16)>,
}

fn get<'a, T>(pool: &'a [T], index: u16, what: &'static str) -> Result<&'a T, Error> {
    pool.get(usize::from(index)).ok_or(Error::OutOfBounds {
        what,
        index: usize::from(index),
        len: pool.len(),
    })
}

impl CompiledModule {
    /// Identifier by index (B-IDX-1).
    pub fn identifier(&self, index: u16) -> Result<&str, Error> {
        get(&self.identifiers, index, "identifier").map(String::as_str)
    }

    /// Address by index.
    pub fn address(&self, index: u16) -> Result<Address, Error> {
        get(&self.addresses, index, "address").copied()
    }

    /// Module handle by index.
    pub fn module_handle(&self, index: u16) -> Result<&ModuleHandle, Error> {
        get(&self.module_handles, index, "module handle")
    }

    /// Datatype handle by index.
    pub fn datatype_handle(&self, index: u16) -> Result<&DatatypeHandle, Error> {
        get(&self.datatype_handles, index, "datatype handle")
    }

    /// Function handle by index.
    pub fn function_handle(&self, index: u16) -> Result<&FunctionHandle, Error> {
        get(&self.function_handles, index, "function handle")
    }

    /// Signature by index.
    pub fn signature(&self, index: u16) -> Result<&[SignatureToken], Error> {
        get(&self.signatures, index, "signature").map(Vec::as_slice)
    }

    /// `(address, name)` of a module handle.
    pub fn module_id(&self, handle: &ModuleHandle) -> Result<(Address, &str), Error> {
        Ok((self.address(handle.address)?, self.identifier(handle.name)?))
    }

    /// This module's `(address, name)`.
    pub fn self_id(&self) -> Result<(Address, &str), Error> {
        self.module_id(self.module_handle(self.self_handle)?)
    }
}
