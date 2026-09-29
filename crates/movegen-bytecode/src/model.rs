//! The ABI tables of a compiled module, borrowed from the bytecode.
//!
//! Zero-copy: identifiers, constants and every type signature are slices of the input. Types keep
//! their bytecode encoding (a pre-order serialization, validated once while reading) and are
//! walked on demand without allocating. Per-item lists (type parameters, fields, variants) live
//! in shared arenas addressed by ranges. Indices are raw `u16`s; accessors bounds-check (B-IDX-1).

use std::ops::Range;

use crate::error::Error;
use crate::sig::Tokens;

/// A 32-byte account address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Address(pub [u8; 32]);

impl Address {
    /// `0x` + 64 lowercase hex digits.
    pub fn to_hex(&self) -> String {
        let mut out = String::with_capacity(66);
        out.push_str("0x");
        out.push_str(&hex::encode(self.0));
        out
    }

    /// Parses `0x`-prefixed or bare hex, 1–64 digits.
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.strip_prefix("0x").unwrap_or(s);
        if s.is_empty() || s.len() > 64 {
            return None;
        }
        let bytes = hex::decode(format!("{s:0>64}")).ok()?;
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
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        [
            (Self::COPY, "copy"),
            (Self::DROP, "drop"),
            (Self::STORE, "store"),
            (Self::KEY, "key"),
        ]
        .into_iter()
        .filter(move |(bit, _)| self.has(*bit))
        .map(|(_, name)| name)
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
    /// Range into [`CompiledModule::datatype_type_parameters`].
    pub type_parameters: Range<u32>,
}

impl DatatypeHandle {
    /// Number of type parameters.
    pub fn arity(&self) -> usize {
        (self.type_parameters.end - self.type_parameters.start) as usize
    }
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
    /// Range into [`CompiledModule::function_type_parameters`].
    pub type_parameters: Range<u32>,
}

impl FunctionHandle {
    /// Number of type parameters.
    pub fn arity(&self) -> usize {
        (self.type_parameters.end - self.type_parameters.start) as usize
    }
}

/// One signature-token tree in its bytecode encoding (validated, B-SIG-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token<'a>(pub(crate) &'a [u8]);

/// A signature: a sequence of tokens in bytecode encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Signature<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) len: u8,
}

impl<'a> Signature<'a> {
    /// Number of tokens.
    pub fn len(&self) -> usize {
        usize::from(self.len)
    }

    /// Whether the signature is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The tokens, in order.
    pub fn tokens(&self) -> Tokens<'a> {
        Tokens::new(self.bytes, self.len())
    }
}

/// A constant pool entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Constant<'a> {
    /// Constant type.
    pub type_: Token<'a>,
    /// BCS-encoded value.
    pub data: &'a [u8],
}

/// A field definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldDefinition<'a> {
    /// Name identifier.
    pub name: u16,
    /// Field type.
    pub type_: Token<'a>,
}

/// A struct definition (B-DEF-1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructDefinition {
    /// Datatype handle.
    pub handle: u16,
    /// Range into [`CompiledModule::fields`]; `None` for native structs.
    pub fields: Option<Range<u32>>,
}

/// An enum variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantDefinition {
    /// Name identifier.
    pub name: u16,
    /// Range into [`CompiledModule::fields`].
    pub fields: Range<u32>,
}

/// An enum definition (B-DEF-1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumDefinition {
    /// Datatype handle.
    pub handle: u16,
    /// Range into [`CompiledModule::variants`], in tag order.
    pub variants: Range<u32>,
}

impl EnumDefinition {
    /// Number of variants.
    pub fn variant_count(&self) -> usize {
        (self.variants.end - self.variants.start) as usize
    }
}

/// A function definition (B-FN-1). Code is bounds-checked while reading and not kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FunctionDefinition {
    /// Function handle.
    pub handle: u16,
    /// Visibility.
    pub visibility: Visibility,
    /// `entry`.
    pub is_entry: bool,
    /// `native` (no code).
    pub is_native: bool,
}

/// `(definition or handle, signature)` pairs of the instantiation tables.
pub type Instantiation = (u16, u16);

/// The ABI-relevant tables of a compiled module, borrowing from its bytecode.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompiledModule<'a> {
    /// Bytecode version (6 or 7).
    pub version: u32,
    /// The module's own handle.
    pub self_handle: u16,
    /// Module handles.
    pub module_handles: Vec<ModuleHandle>,
    /// Datatype handles.
    pub datatype_handles: Vec<DatatypeHandle>,
    /// Arena of datatype type parameters.
    pub datatype_type_parameters: Vec<DatatypeTypeParameter>,
    /// Function handles.
    pub function_handles: Vec<FunctionHandle>,
    /// Arena of function type-parameter constraints.
    pub function_type_parameters: Vec<AbilitySet>,
    /// Signature pool.
    pub signatures: Vec<Signature<'a>>,
    /// Identifier pool.
    pub identifiers: Vec<&'a str>,
    /// Address pool (raw 32-byte entries).
    pub addresses: &'a [u8],
    /// Constant pool.
    pub constants: Vec<Constant<'a>>,
    /// Struct definitions, definition order.
    pub struct_defs: Vec<StructDefinition>,
    /// Enum definitions, definition order.
    pub enum_defs: Vec<EnumDefinition>,
    /// Arena of enum variants.
    pub variants: Vec<VariantDefinition>,
    /// Arena of struct and variant fields.
    pub fields: Vec<FieldDefinition<'a>>,
    /// Function definitions, definition order.
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

fn get<'s, T>(pool: &'s [T], index: u16, what: &'static str) -> Result<&'s T, Error> {
    pool.get(usize::from(index)).ok_or(Error::OutOfBounds {
        what,
        index: usize::from(index),
        len: pool.len(),
    })
}

fn range<'s, T>(arena: &'s [T], r: &Range<u32>) -> &'s [T] {
    &arena[r.start as usize..r.end as usize]
}

impl<'a> CompiledModule<'a> {
    /// Number of addresses in the pool.
    pub fn address_count(&self) -> usize {
        self.addresses.len() / 32
    }

    /// Identifier by index (B-IDX-1).
    pub fn identifier(&self, index: u16) -> Result<&'a str, Error> {
        get(&self.identifiers, index, "identifier").copied()
    }

    /// Address by index.
    pub fn address(&self, index: u16) -> Result<Address, Error> {
        let i = usize::from(index);
        if i >= self.address_count() {
            return Err(Error::OutOfBounds {
                what: "address",
                index: i,
                len: self.address_count(),
            });
        }
        Ok(Address(
            self.addresses[i * 32..i * 32 + 32]
                .try_into()
                .expect("32 bytes"),
        ))
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
    pub fn signature(&self, index: u16) -> Result<Signature<'a>, Error> {
        get(&self.signatures, index, "signature").copied()
    }

    /// A datatype handle's type parameters.
    pub fn datatype_type_params(&self, handle: &DatatypeHandle) -> &[DatatypeTypeParameter] {
        range(&self.datatype_type_parameters, &handle.type_parameters)
    }

    /// A function handle's type-parameter constraints.
    pub fn function_type_params(&self, handle: &FunctionHandle) -> &[AbilitySet] {
        range(&self.function_type_parameters, &handle.type_parameters)
    }

    /// A struct's fields (`None` if native).
    pub fn struct_fields(&self, def: &StructDefinition) -> Option<&[FieldDefinition<'a>]> {
        def.fields.as_ref().map(|r| range(&self.fields, r))
    }

    /// An enum's variants, in tag order.
    pub fn enum_variants(&self, def: &EnumDefinition) -> &[VariantDefinition] {
        range(&self.variants, &def.variants)
    }

    /// A variant's fields.
    pub fn variant_fields(&self, variant: &VariantDefinition) -> &[FieldDefinition<'a>] {
        range(&self.fields, &variant.fields)
    }

    /// `(address, name)` of a module handle.
    pub fn module_id(&self, handle: &ModuleHandle) -> Result<(Address, &'a str), Error> {
        Ok((self.address(handle.address)?, self.identifier(handle.name)?))
    }

    /// This module's `(address, name)`.
    pub fn self_id(&self) -> Result<(Address, &'a str), Error> {
        self.module_id(self.module_handle(self.self_handle)?)
    }
}
