//! Reader errors.

/// Why a module (or package) could not be read.
///
/// Each variant names the spec rule it enforces (see `docs/SPEC.md`).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// B-HDR-1: not a publishable Move module.
    #[error("bad magic: expected a1 1c eb 0b")]
    BadMagic,
    /// B-HDR-2: unsupported or unflavored version.
    #[error("unsupported bytecode version {version} (flavor {flavor:?}); supported: 6, 7")]
    UnsupportedVersion {
        /// Decoded version.
        version: u32,
        /// Decoded flavor byte, if any.
        flavor: Option<u8>,
    },
    /// Ran out of bytes.
    #[error("unexpected end of input while reading {0}")]
    Eof(&'static str),
    /// B-ULEB-1: non-canonical or overflowing ULEB128.
    #[error("invalid ULEB128 while reading {0}")]
    BadUleb(&'static str),
    /// B-ULEB-1: a value exceeded its field's maximum.
    #[error("{what} {value} exceeds maximum {max}")]
    TooLarge {
        /// Field being read.
        what: &'static str,
        /// Value read.
        value: u64,
        /// Allowed maximum.
        max: u64,
    },
    /// B-TBL-1: unknown table kind.
    #[error("unknown table kind {0:#x}")]
    UnknownTable(u8),
    /// B-TBL-2: tables overlap, have gaps, are empty, repeat, or exceed the binary.
    #[error("malformed table header: {0}")]
    BadTableHeader(&'static str),
    /// B-TBL-3: a table's entries don't exactly fill its byte range.
    #[error("table {0:?} has trailing or truncated entries")]
    BadTableContents(TableKind),
    /// B-TBL-3: an enum table in a pre-v7 module.
    #[error("enum tables require bytecode version 7")]
    EnumsBeforeV7,
    /// B-END-1: bytes after the module.
    #[error("{0} trailing bytes after module")]
    TrailingBytes(usize),
    /// B-ID-1: invalid identifier.
    #[error("invalid identifier {0:?}")]
    BadIdentifier(String),
    /// B-ADDR-1: address pool size not a multiple of 32.
    #[error("address pool size {0} is not a multiple of 32")]
    BadAddressPool(u32),
    /// B-SIG-1: malformed signature token.
    #[error("malformed signature token: {0}")]
    BadSignature(&'static str),
    /// B-ABIL-1: unknown ability bits.
    #[error("invalid ability set {0:#x}")]
    BadAbilities(u64),
    /// B-DEF-1 / B-FN-1 / B-CODE-1: bad flag byte.
    #[error("invalid {what} flag {value:#x}")]
    BadFlag {
        /// Which flag.
        what: &'static str,
        /// Its value.
        value: u8,
    },
    /// B-FN-1 / B-CODE-1: global storage is not supported on Sui.
    #[error("global storage operations are not supported")]
    GlobalStorage,
    /// B-CODE-1: unknown opcode.
    #[error("unknown opcode {0:#x}")]
    UnknownOpcode(u8),
    /// B-IDX-1: an index points outside its table.
    #[error("{what} index {index} out of bounds (len {len})")]
    OutOfBounds {
        /// Which table.
        what: &'static str,
        /// Index read.
        index: usize,
        /// Table length.
        len: usize,
    },
    /// B-ABI-1: two datatypes, functions, variants (in one enum) or fields (in one struct or
    /// variant) with the same name — a verifier-level fault that Mysten's deserializer accepts
    /// and its normalized view silently collapses; bindings must never see it.
    #[error("duplicate {what} definition {name:?}")]
    Duplicate {
        /// `datatype`, `function`, `variant` or `field`.
        what: &'static str,
        /// The repeated name.
        name: String,
    },
    /// B-ABI-2: a definition refers to a handle owned by another module.
    #[error("{what} definition {name:?} belongs to another module")]
    ForeignDefinition {
        /// `datatype` or `function`.
        what: &'static str,
        /// The definition's name.
        name: String,
    },
    /// B-ABI-3: a datatype is instantiated with the wrong number of type arguments, or a type
    /// parameter index is out of range for its context.
    #[error("ill-formed type: {0}")]
    IllFormedType(String),
    /// B-PKG-1: the package object is not valid BCS.
    #[error("invalid package BCS: {0}")]
    BadPackage(String),
    /// A module inside a package failed to read.
    #[error("module {module}: {source}")]
    InModule {
        /// Module name (from the package's module map).
        module: String,
        /// Underlying error.
        source: Box<Error>,
    },
}

/// Table kinds (B-TBL-1), `file_format_common.rs` `TableType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[allow(missing_docs)]
pub enum TableKind {
    ModuleHandles = 0x1,
    DatatypeHandles = 0x2,
    FunctionHandles = 0x3,
    FunctionInst = 0x4,
    Signatures = 0x5,
    ConstantPool = 0x6,
    Identifiers = 0x7,
    AddressIdentifiers = 0x8,
    StructDefs = 0xA,
    StructDefInst = 0xB,
    FunctionDefs = 0xC,
    FieldHandle = 0xD,
    FieldInst = 0xE,
    FriendDecls = 0xF,
    Metadata = 0x10,
    EnumDefs = 0x11,
    EnumDefInst = 0x12,
    VariantHandles = 0x13,
    VariantInstHandles = 0x14,
}

impl TableKind {
    pub(crate) fn from_u8(value: u8) -> Result<Self, Error> {
        use TableKind as T;
        Ok(match value {
            0x1 => T::ModuleHandles,
            0x2 => T::DatatypeHandles,
            0x3 => T::FunctionHandles,
            0x4 => T::FunctionInst,
            0x5 => T::Signatures,
            0x6 => T::ConstantPool,
            0x7 => T::Identifiers,
            0x8 => T::AddressIdentifiers,
            0xA => T::StructDefs,
            0xB => T::StructDefInst,
            0xC => T::FunctionDefs,
            0xD => T::FieldHandle,
            0xE => T::FieldInst,
            0xF => T::FriendDecls,
            0x10 => T::Metadata,
            0x11 => T::EnumDefs,
            0x12 => T::EnumDefInst,
            0x13 => T::VariantHandles,
            0x14 => T::VariantInstHandles,
            other => return Err(Error::UnknownTable(other)),
        })
    }

    pub(crate) fn is_enum_table(self) -> bool {
        matches!(
            self,
            TableKind::EnumDefs
                | TableKind::EnumDefInst
                | TableKind::VariantHandles
                | TableKind::VariantInstHandles
        )
    }
}
