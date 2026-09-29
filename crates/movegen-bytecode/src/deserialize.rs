//! Module deserialization, mirroring `move-binary-format`'s `deserializer.rs` at Sui
//! `mainnet-v1.80.1` for the tables in [`CompiledModule`]. Rule ids refer to `docs/SPEC.md`.

use std::collections::HashSet;

use crate::cursor::Cursor;
use crate::error::{Error, TableKind};
use crate::model::{
    AbilitySet, Address, CodeUnit, CompiledModule, Constant, DatatypeHandle, DatatypeTypeParameter,
    EnumDefinition, FieldDefinition, FunctionDefinition, FunctionHandle, ModuleHandle, Operand,
    SignatureToken, StructDefinition, VariantDefinition, Visibility,
};

const MAGIC: [u8; 4] = [0xA1, 0x1C, 0xEB, 0x0B];
const VERSION_6: u32 = 6;
const VERSION_7: u32 = 7;
const SUI_FLAVOR: u8 = 0x05;

const TABLE_COUNT_MAX: u64 = 255;
const U32_MAX: u64 = 0xffff_ffff;
const IDENTIFIER_SIZE_MAX: u64 = 65535;
const CONSTANT_SIZE_MAX: u64 = 65535;
const SIGNATURE_SIZE_MAX: u64 = 255;
const TYPE_PARAMETER_COUNT_MAX: u64 = 255;
const TYPE_PARAMETER_INDEX_MAX: u64 = 65536;
const FIELD_COUNT_MAX: u64 = 255;
const FIELD_OFFSET_MAX: u64 = 255;
const ACQUIRES_COUNT_MAX: u64 = 255;
const VARIANT_COUNT_MAX: u64 = 127;
const VARIANT_TAG_MAX_VALUE: u64 = VARIANT_COUNT_MAX - 1;
const VARIANT_HANDLE_INDEX_MAX: u64 = 1024;
const JUMP_TABLE_INDEX_MAX: u64 = 127;
const BYTECODE_COUNT_MAX: u64 = 65535;
const BYTECODE_INDEX_MAX: u64 = 65535;
const LOCAL_INDEX_MAX: u64 = 255;
const SIGNATURE_TOKEN_DEPTH_MAX: usize = 256;

struct Table {
    kind: TableKind,
    offset: u32,
    count: u32,
}

/// Reads one compiled module (B-HDR-1 … B-CODE-1).
pub fn deserialize(binary: &[u8]) -> Result<CompiledModule, Error> {
    let mut cursor = Cursor::new(binary);

    // B-HDR-1
    if cursor.array::<4>("magic").ok() != Some(MAGIC) {
        return Err(Error::BadMagic);
    }
    // B-HDR-2
    let flavored = cursor.u32_le("version")?;
    let (version, flavor) = if flavored <= VERSION_6 {
        (flavored, None)
    } else {
        // Truncation to the flavor byte is intended.
        #[allow(clippy::cast_possible_truncation)]
        (flavored & 0x00ff_ffff, Some((flavored >> 24) as u8))
    };
    if !(VERSION_6..=VERSION_7).contains(&version)
        || (version >= VERSION_7 && flavor != Some(SUI_FLAVOR))
    {
        return Err(Error::UnsupportedVersion { version, flavor });
    }

    // B-TBL-1
    let table_count = cursor.uleb_max("table count", TABLE_COUNT_MAX)?;
    let mut tables = Vec::new();
    for _ in 0..table_count {
        let kind = TableKind::from_u8(cursor.u8("table header")?)?;
        let offset = u32::try_from(cursor.uleb_max("table offset", U32_MAX)?).expect("bounded");
        let count = u32::try_from(cursor.uleb_max("table size", U32_MAX)?).expect("bounded");
        tables.push(Table {
            kind,
            offset,
            count,
        });
    }

    // B-TBL-2 (`check_tables`)
    tables.sort_by_key(|t| t.offset);
    let mut end: u32 = 0;
    let mut kinds = HashSet::new();
    for table in &tables {
        if table.offset != end {
            return Err(Error::BadTableHeader("tables are not contiguous"));
        }
        if table.count == 0 {
            return Err(Error::BadTableHeader("empty table"));
        }
        end = end
            .checked_add(table.count)
            .ok_or(Error::BadTableHeader("table size overflow"))?;
        if !kinds.insert(table.kind) {
            return Err(Error::BadTableHeader("duplicate table"));
        }
        if end as usize > binary.len() {
            return Err(Error::BadTableHeader("table exceeds binary"));
        }
    }
    let data_offset = cursor.position();
    if data_offset as u64 + u64::from(end) > binary.len() as u64 {
        return Err(Error::BadTableHeader("table size too big"));
    }
    let data = &binary[data_offset..data_offset + end as usize];

    // B-END-1
    let mut tail = Cursor::new(&binary[data_offset + end as usize..]);
    let self_handle = tail.index("self module handle")?;
    if !tail.is_empty() {
        return Err(Error::TrailingBytes(tail.remaining()));
    }

    let mut module = CompiledModule {
        version,
        self_handle,
        module_handles: Vec::new(),
        datatype_handles: Vec::new(),
        function_handles: Vec::new(),
        signatures: Vec::new(),
        identifiers: Vec::new(),
        addresses: Vec::new(),
        constants: Vec::new(),
        struct_defs: Vec::new(),
        enum_defs: Vec::new(),
        function_defs: Vec::new(),
        friends: Vec::new(),
        function_instantiations: Vec::new(),
        struct_instantiations: Vec::new(),
        enum_instantiations: Vec::new(),
        field_handles: Vec::new(),
        field_instantiations: Vec::new(),
        variant_handles: Vec::new(),
        variant_instantiation_handles: Vec::new(),
    };

    for table in &tables {
        // B-TBL-3
        if table.kind.is_enum_table() && version < VERSION_7 {
            return Err(Error::EnumsBeforeV7);
        }
        let bytes = &data[table.offset as usize..(table.offset + table.count) as usize];
        read_table(&mut module, table.kind, bytes, version)?;
    }
    crate::bounds::check(&module)?;
    Ok(module)
}

/// Reads a whole table; entries must exactly fill `bytes` (B-TBL-3).
fn read_table(
    module: &mut CompiledModule,
    kind: TableKind,
    bytes: &[u8],
    version: u32,
) -> Result<(), Error> {
    if kind == TableKind::AddressIdentifiers {
        // B-ADDR-1
        if !bytes.len().is_multiple_of(32) {
            let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
            return Err(Error::BadAddressPool(len));
        }
        module.addresses = bytes
            .chunks_exact(32)
            .map(|c| Address(c.try_into().expect("32-byte chunk")))
            .collect();
        return Ok(());
    }
    let mut c = Cursor::new(bytes);
    while !c.is_empty() {
        match kind {
            TableKind::ModuleHandles => module.module_handles.push(module_handle(&mut c)?),
            TableKind::FriendDecls => module.friends.push(module_handle(&mut c)?),
            TableKind::DatatypeHandles => module.datatype_handles.push(datatype_handle(&mut c)?),
            TableKind::FunctionHandles => module.function_handles.push(function_handle(&mut c)?),
            TableKind::Signatures => module.signatures.push(signature(&mut c)?),
            TableKind::Identifiers => module.identifiers.push(identifier(&mut c)?),
            TableKind::ConstantPool => module.constants.push(constant(&mut c)?),
            TableKind::StructDefs => module.struct_defs.push(struct_def(&mut c)?),
            TableKind::EnumDefs => module.enum_defs.push(enum_def(&mut c)?),
            TableKind::FunctionDefs => module.function_defs.push(function_def(&mut c, version)?),
            // Kept for bounds checking; code-only tables.
            TableKind::FunctionInst => module.function_instantiations.push(instantiation(&mut c)?),
            TableKind::StructDefInst => module.struct_instantiations.push(instantiation(&mut c)?),
            TableKind::EnumDefInst => module.enum_instantiations.push(instantiation(&mut c)?),
            TableKind::FieldInst => module.field_instantiations.push(instantiation(&mut c)?),
            TableKind::FieldHandle => {
                let owner = c.index("struct definition")?;
                let field =
                    u16::try_from(c.uleb_max("field offset", FIELD_OFFSET_MAX)?).expect("small");
                module.field_handles.push((owner, field));
            }
            TableKind::VariantHandles => module.variant_handles.push(variant_handle(&mut c)?),
            TableKind::VariantInstHandles => {
                module
                    .variant_instantiation_handles
                    .push(variant_handle(&mut c)?);
            }
            // Sui's binary config rejects extraneous bytes, which disallows metadata tables.
            TableKind::Metadata => return Err(Error::BadTableContents(kind)),
            TableKind::AddressIdentifiers => unreachable!("handled above"),
        }
    }
    Ok(())
}

fn instantiation(c: &mut Cursor<'_>) -> Result<(u16, u16), Error> {
    Ok((c.index("definition or handle")?, c.index("signature")?))
}

fn variant_handle(c: &mut Cursor<'_>) -> Result<(u16, u16), Error> {
    let owner = c.uleb_max("variant handle owner", u64::from(u16::MAX))?;
    let tag = c.uleb_max("variant tag", VARIANT_TAG_MAX_VALUE)?;
    Ok((
        u16::try_from(owner).expect("bounded"),
        u16::try_from(tag).expect("bounded"),
    ))
}

fn module_handle(c: &mut Cursor<'_>) -> Result<ModuleHandle, Error> {
    Ok(ModuleHandle {
        address: c.index("address")?,
        name: c.index("identifier")?,
    })
}

/// B-ABIL-1
fn abilities(c: &mut Cursor<'_>) -> Result<AbilitySet, Error> {
    let value = c.uleb("ability set")?;
    if value > 0xF {
        return Err(Error::BadAbilities(value));
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(AbilitySet(value as u8))
}

fn type_parameter_count(c: &mut Cursor<'_>) -> Result<u64, Error> {
    c.uleb_max("type parameter count", TYPE_PARAMETER_COUNT_MAX)
}

/// B-TP-1
fn datatype_handle(c: &mut Cursor<'_>) -> Result<DatatypeHandle, Error> {
    let module = c.index("module handle")?;
    let name = c.index("identifier")?;
    let abilities = abilities(c)?;
    let count = type_parameter_count(c)?;
    let mut type_parameters = Vec::new();
    for _ in 0..count {
        let constraints = self::abilities(c)?;
        let is_phantom = c.uleb_max("phantom flag", 1)? != 0;
        type_parameters.push(DatatypeTypeParameter {
            constraints,
            is_phantom,
        });
    }
    Ok(DatatypeHandle {
        module,
        name,
        abilities,
        type_parameters,
    })
}

fn function_handle(c: &mut Cursor<'_>) -> Result<FunctionHandle, Error> {
    let module = c.index("module handle")?;
    let name = c.index("identifier")?;
    let parameters = c.index("signature")?;
    let return_ = c.index("signature")?;
    let count = type_parameter_count(c)?;
    let mut type_parameters = Vec::new();
    for _ in 0..count {
        type_parameters.push(abilities(c)?);
    }
    Ok(FunctionHandle {
        module,
        name,
        parameters,
        return_,
        type_parameters,
    })
}

/// B-ID-1: `[a-zA-Z][a-zA-Z0-9_]*` or `_[a-zA-Z0-9_]+`.
pub(crate) fn is_valid_identifier(s: &str) -> bool {
    let b = s.as_bytes();
    let tail_ok = |from: usize| {
        b[from..]
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b'_')
    };
    match b.first() {
        Some(c) if c.is_ascii_alphabetic() => tail_ok(1),
        Some(b'_') if b.len() > 1 => tail_ok(1),
        _ => false,
    }
}

fn identifier(c: &mut Cursor<'_>) -> Result<String, Error> {
    let len = usize::try_from(c.uleb_max("identifier size", IDENTIFIER_SIZE_MAX)?).expect("small");
    let bytes = c.take(len, "identifier")?;
    let s = std::str::from_utf8(bytes)
        .map_err(|_| Error::BadIdentifier(String::from_utf8_lossy(bytes).into_owned()))?;
    if !is_valid_identifier(s) {
        return Err(Error::BadIdentifier(s.to_owned()));
    }
    Ok(s.to_owned())
}

fn constant(c: &mut Cursor<'_>) -> Result<Constant, Error> {
    let type_ = signature_token(c)?;
    let len = usize::try_from(c.uleb_max("constant size", CONSTANT_SIZE_MAX)?).expect("small");
    let data = c.take(len, "constant")?.to_vec();
    Ok(Constant { type_, data })
}

fn signature(c: &mut Cursor<'_>) -> Result<Vec<SignatureToken>, Error> {
    let len = c.uleb_max("signature size", SIGNATURE_SIZE_MAX)?;
    (0..len).map(|_| signature_token(c)).collect()
}

/// B-SIG-1, a port of `load_signature_token`'s explicit-stack algorithm (same depth limit).
pub(crate) fn signature_token(c: &mut Cursor<'_>) -> Result<SignatureToken, Error> {
    enum Builder {
        Saturated(SignatureToken),
        Vector,
        Reference,
        MutableReference,
        Inst {
            handle: u16,
            arity: usize,
            args: Vec<SignatureToken>,
        },
    }
    impl Builder {
        fn apply(self, tok: SignatureToken) -> Builder {
            match self {
                Builder::Vector => Builder::Saturated(SignatureToken::Vector(Box::new(tok))),
                Builder::Reference => Builder::Saturated(SignatureToken::Reference(Box::new(tok))),
                Builder::MutableReference => {
                    Builder::Saturated(SignatureToken::MutableReference(Box::new(tok)))
                }
                Builder::Inst {
                    handle,
                    arity,
                    mut args,
                } => {
                    args.push(tok);
                    if args.len() >= arity {
                        Builder::Saturated(SignatureToken::DatatypeInstantiation(handle, args))
                    } else {
                        Builder::Inst {
                            handle,
                            arity,
                            args,
                        }
                    }
                }
                Builder::Saturated(_) => unreachable!("saturated tokens are never applied to"),
            }
        }
    }
    let next = |c: &mut Cursor<'_>| -> Result<Builder, Error> {
        Ok(match c.u8("signature token")? {
            0x1 => Builder::Saturated(SignatureToken::Bool),
            0x2 => Builder::Saturated(SignatureToken::U8),
            0x3 => Builder::Saturated(SignatureToken::U64),
            0x4 => Builder::Saturated(SignatureToken::U128),
            0x5 => Builder::Saturated(SignatureToken::Address),
            0x6 => Builder::Reference,
            0x7 => Builder::MutableReference,
            0x8 => Builder::Saturated(SignatureToken::Datatype(c.index("datatype handle")?)),
            0x9 => {
                let idx = c.uleb_max("type parameter index", TYPE_PARAMETER_INDEX_MAX)?;
                let idx = u16::try_from(idx)
                    .map_err(|_| Error::BadSignature("type parameter index overflow"))?;
                Builder::Saturated(SignatureToken::TypeParameter(idx))
            }
            0xA => Builder::Vector,
            0xB => {
                let handle = c.index("datatype handle")?;
                let arity = usize::try_from(type_parameter_count(c)?).expect("small");
                if arity == 0 {
                    return Err(Error::BadSignature("datatype instantiation with arity 0"));
                }
                Builder::Inst {
                    handle,
                    arity,
                    args: Vec::new(),
                }
            }
            0xC => Builder::Saturated(SignatureToken::Signer),
            0xD => Builder::Saturated(SignatureToken::U16),
            0xE => Builder::Saturated(SignatureToken::U32),
            0xF => Builder::Saturated(SignatureToken::U256),
            _ => return Err(Error::BadSignature("unknown token")),
        })
    };
    let mut stack = match next(c)? {
        Builder::Saturated(tok) => return Ok(tok),
        builder => vec![builder],
    };
    loop {
        if stack.len() > SIGNATURE_TOKEN_DEPTH_MAX {
            return Err(Error::BadSignature("maximum nesting depth reached"));
        }
        if let Some(Builder::Saturated(_)) = stack.last() {
            let Some(Builder::Saturated(tok)) = stack.pop() else {
                unreachable!()
            };
            match stack.pop() {
                Some(parent) => stack.push(parent.apply(tok)),
                None => return Ok(tok),
            }
        } else {
            stack.push(next(c)?);
        }
    }
}

fn fields(c: &mut Cursor<'_>) -> Result<Vec<FieldDefinition>, Error> {
    let count = c.uleb_max("field count", FIELD_COUNT_MAX)?;
    (0..count)
        .map(|_| {
            Ok(FieldDefinition {
                name: c.index("identifier")?,
                type_: signature_token(c)?,
            })
        })
        .collect()
}

/// B-DEF-1
fn struct_def(c: &mut Cursor<'_>) -> Result<StructDefinition, Error> {
    let handle = c.index("datatype handle")?;
    let fields = match c.u8("struct flag")? {
        0x1 => None,
        0x2 => Some(fields(c)?),
        value => {
            return Err(Error::BadFlag {
                what: "struct",
                value,
            });
        }
    };
    Ok(StructDefinition { handle, fields })
}

/// B-DEF-1
fn enum_def(c: &mut Cursor<'_>) -> Result<EnumDefinition, Error> {
    let handle = c.index("datatype handle")?;
    match c.u8("enum flag")? {
        0x2 => {}
        value => {
            return Err(Error::BadFlag {
                what: "enum",
                value,
            });
        }
    }
    let count = c.uleb_max("variant count", VARIANT_COUNT_MAX)?;
    if count == 0 {
        return Err(Error::BadSignature("enum with no variants"));
    }
    let variants = (0..count)
        .map(|_| {
            Ok(VariantDefinition {
                name: c.index("identifier")?,
                fields: fields(c)?,
            })
        })
        .collect::<Result<_, Error>>()?;
    Ok(EnumDefinition { handle, variants })
}

const FLAG_NATIVE: u8 = 0b10;
const FLAG_ENTRY: u8 = 0b100;

/// B-FN-1 (bytecode version ≥ 5 layout).
fn function_def(c: &mut Cursor<'_>, version: u32) -> Result<FunctionDefinition, Error> {
    let handle = c.index("function handle")?;
    let visibility = match c.u8("visibility")? {
        0x0 => Visibility::Private,
        0x1 => Visibility::Public,
        0x3 => Visibility::Friend,
        value => {
            return Err(Error::BadFlag {
                what: "visibility",
                value,
            });
        }
    };
    let mut flags = c.u8("function flags")?;
    let is_entry = flags & FLAG_ENTRY != 0;
    flags &= !FLAG_ENTRY;
    let acquires = c.uleb_max("acquires count", ACQUIRES_COUNT_MAX)?;
    if acquires != 0 {
        return Err(Error::GlobalStorage);
    }
    let is_native = flags & FLAG_NATIVE != 0;
    flags &= !FLAG_NATIVE;
    let code = if is_native {
        None
    } else {
        Some(code_unit(c, version)?)
    };
    if flags != 0 {
        return Err(Error::BadFlag {
            what: "function",
            value: flags,
        });
    }
    Ok(FunctionDefinition {
        handle,
        visibility,
        is_entry,
        is_native,
        code,
    })
}

/// B-CODE-1: reads a code unit, keeping only table-indexing operands.
fn code_unit(c: &mut Cursor<'_>, version: u32) -> Result<CodeUnit, Error> {
    let locals = c.index("locals signature")?;
    let count = c.uleb_max("bytecode count", BYTECODE_COUNT_MAX)?;
    let code = (0..count)
        .map(|_| instruction(c, version))
        .collect::<Result<Vec<_>, _>>()?;
    let mut jump_tables = Vec::new();
    if version >= VERSION_7 {
        let tables = c.uleb_max("jump table count", JUMP_TABLE_INDEX_MAX)?;
        for _ in 0..tables {
            let head = c.index("enum definition")?;
            let branches = c.uleb_max("jump table branches", VARIANT_COUNT_MAX)?;
            match c.u8("jump table flag")? {
                0x1 => {}
                value => {
                    return Err(Error::BadFlag {
                        what: "jump table",
                        value,
                    });
                }
            }
            let offsets = (0..branches)
                .map(|_| {
                    Ok(
                        u16::try_from(c.uleb_max("bytecode index", BYTECODE_INDEX_MAX)?)
                            .expect("bounded"),
                    )
                })
                .collect::<Result<_, Error>>()?;
            jump_tables.push((head, offsets));
        }
    }
    Ok(CodeUnit {
        locals,
        code,
        jump_tables,
    })
}

fn instruction(c: &mut Cursor<'_>, version: u32) -> Result<Operand, Error> {
    let opcode = c.u8("opcode")?;
    let enums = || {
        if version < VERSION_7 {
            Err(Error::EnumsBeforeV7)
        } else {
            Ok(())
        }
    };
    let u16_of = |v: u64| u16::try_from(v).expect("bounded by uleb_max");
    Ok(match opcode {
        // No operands.
        0x01
        | 0x02
        | 0x08
        | 0x09
        | 0x14..=0x28
        | 0x2E
        | 0x2F
        | 0x30
        | 0x33..=0x35
        | 0x4B..=0x4D => Operand::None,
        0x03..=0x05 => Operand::Branch(u16_of(c.uleb_max("bytecode index", BYTECODE_INDEX_MAX)?)),
        0x06 => {
            c.take(8, "u64")?;
            Operand::None
        }
        0x07 => Operand::Constant(c.index("constant")?),
        0x0A..=0x0E => {
            let local = c.uleb_max("local index", LOCAL_INDEX_MAX)?;
            Operand::Local(u8::try_from(local).expect("bounded"))
        }
        0x0F | 0x10 => Operand::FieldHandle(c.index("field handle")?),
        0x36 | 0x37 => Operand::FieldInstantiation(c.index("field instantiation")?),
        0x11 => Operand::FunctionHandle(c.index("function handle")?),
        0x38 => Operand::FunctionInstantiation(c.index("function instantiation")?),
        0x12 | 0x13 => Operand::StructDefinition(c.index("struct definition")?),
        0x39 | 0x3A => Operand::StructInstantiation(c.index("struct instantiation")?),
        0x31 => {
            c.u8("u8")?;
            Operand::None
        }
        0x32 => {
            c.take(16, "u128")?;
            Operand::None
        }
        // VEC_PACK / VEC_UNPACK: signature + u64 count.
        0x40 | 0x46 => {
            let sig = c.index("signature")?;
            c.take(8, "u64")?;
            Operand::VectorSignature(sig)
        }
        0x41..=0x45 | 0x47 => Operand::VectorSignature(c.index("signature")?),
        0x48 => {
            c.take(2, "u16")?;
            Operand::None
        }
        0x49 => {
            c.take(4, "u32")?;
            Operand::None
        }
        0x4A => {
            c.take(32, "u256")?;
            Operand::None
        }
        0x4E | 0x50..=0x52 => {
            enums()?;
            Operand::VariantHandle(u16_of(
                c.uleb_max("variant handle", VARIANT_HANDLE_INDEX_MAX)?,
            ))
        }
        0x4F | 0x53..=0x55 => {
            enums()?;
            Operand::VariantInstantiation(u16_of(
                c.uleb_max("variant instantiation handle", VARIANT_HANDLE_INDEX_MAX)?,
            ))
        }
        0x56 => {
            enums()?;
            Operand::JumpTable(u16_of(
                c.uleb_max("jump table index", JUMP_TABLE_INDEX_MAX)?,
            ))
        }
        // Deprecated global storage.
        0x29..=0x2D | 0x3B..=0x3F => return Err(Error::GlobalStorage),
        other => return Err(Error::UnknownOpcode(other)),
    })
}
