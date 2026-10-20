//! Module deserialization, mirroring `move-binary-format`'s `deserializer.rs` at Sui
//! `testnet-v1.82.1` for the tables in [`CompiledModule`]. Rule ids refer to `docs/SPEC.md`.
//!
//! Zero-copy: everything borrows from the input. Tables are read in a fixed order — every table
//! but function definitions, then the table-level bounds checks, then function definitions,
//! whose code is bounds-checked inline and discarded. The accepted set equals reading all tables
//! first and checking afterwards (as Mysten does); the oracle tests enforce that.

use crate::bounds::{self, CodeChecker};
use crate::cursor::Cursor;
use crate::error::{Error, TableKind};
use crate::model::{
    AbilitySet, CompiledModule, Constant, DatatypeHandle, DatatypeTypeParameter, EnumDefinition,
    FieldDefinition, FunctionDefinition, FunctionHandle, ModuleHandle, Signature, StructDefinition,
    VariantDefinition, Visibility,
};
use crate::sig::read_token;

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

#[derive(Clone, Copy)]
struct Table {
    kind: TableKind,
    offset: u32,
    count: u32,
}

/// Reads one compiled module, borrowing from `binary` (B-HDR-1 … B-CODE-1).
pub fn deserialize(binary: &[u8]) -> Result<CompiledModule<'_>, Error> {
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
        #[allow(clippy::cast_possible_truncation)]
        (flavored & 0x00ff_ffff, Some((flavored >> 24) as u8))
    };
    if !(VERSION_6..=VERSION_7).contains(&version)
        || (version >= VERSION_7 && flavor != Some(SUI_FLAVOR))
    {
        return Err(Error::UnsupportedVersion { version, flavor });
    }

    // B-TBL-1: at most 20 distinct kinds can pass B-TBL-2, so a fixed array suffices; more
    // headers than that must contain a duplicate.
    let table_count = cursor.uleb_max("table count", TABLE_COUNT_MAX)?;
    let mut tables = [Table {
        kind: TableKind::ModuleHandles,
        offset: 0,
        count: 0,
    }; 20];
    let mut n = 0;
    for _ in 0..table_count {
        let kind = TableKind::from_u8(cursor.u8("table header")?)?;
        let offset = u32::try_from(cursor.uleb_max("table offset", U32_MAX)?).expect("bounded");
        let count = u32::try_from(cursor.uleb_max("table size", U32_MAX)?).expect("bounded");
        if n == tables.len() {
            return Err(Error::BadTableHeader("duplicate table"));
        }
        tables[n] = Table {
            kind,
            offset,
            count,
        };
        n += 1;
    }
    let tables = &mut tables[..n];

    // B-TBL-2 (`check_tables`)
    tables.sort_unstable_by_key(|t| t.offset);
    let mut end: u32 = 0;
    let mut seen = 0u32;
    for table in tables.iter() {
        if table.offset != end {
            return Err(Error::BadTableHeader("tables are not contiguous"));
        }
        if table.count == 0 {
            return Err(Error::BadTableHeader("empty table"));
        }
        end = end
            .checked_add(table.count)
            .ok_or(Error::BadTableHeader("table size overflow"))?;
        let bit = 1u32 << (table.kind as u32);
        if seen & bit != 0 {
            return Err(Error::BadTableHeader("duplicate table"));
        }
        seen |= bit;
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
        ..CompiledModule::default()
    };
    let mut function_defs = None;
    for table in tables.iter() {
        // B-TBL-3
        if table.kind.is_enum_table() && version < VERSION_7 {
            return Err(Error::EnumsBeforeV7);
        }
        let bytes = &data[table.offset as usize..(table.offset + table.count) as usize];
        if table.kind == TableKind::FunctionDefs {
            function_defs = Some(bytes);
        } else {
            read_table(&mut module, table.kind, bytes)?;
        }
    }
    // B-IDX-1 for every table but function definitions, then function definitions with their
    // code checked inline.
    let memo = bounds::check_tables(&module)?;
    if let Some(bytes) = function_defs {
        let mut c = Cursor::new(bytes);
        let mut defs = Vec::new();
        {
            let checker = CodeChecker::new(&module, &memo);
            while !c.is_empty() {
                defs.push(function_def(&mut c, version, &checker)?);
            }
        }
        module.function_defs = defs;
    }
    Ok(module)
}

/// Reads a whole table; entries must exactly fill `bytes` (B-TBL-3).
fn read_table<'a>(
    m: &mut CompiledModule<'a>,
    kind: TableKind,
    bytes: &'a [u8],
) -> Result<(), Error> {
    if kind == TableKind::AddressIdentifiers {
        // B-ADDR-1
        if !bytes.len().is_multiple_of(32) {
            return Err(Error::BadAddressPool(
                u32::try_from(bytes.len()).unwrap_or(u32::MAX),
            ));
        }
        m.addresses = bytes;
        return Ok(());
    }
    let mut c = Cursor::new(bytes);
    while !c.is_empty() {
        match kind {
            TableKind::ModuleHandles => m.module_handles.push(module_handle(&mut c)?),
            TableKind::FriendDecls => m.friends.push(module_handle(&mut c)?),
            TableKind::DatatypeHandles => {
                let h = datatype_handle(&mut c, &mut m.datatype_type_parameters)?;
                m.datatype_handles.push(h);
            }
            TableKind::FunctionHandles => {
                let h = function_handle(&mut c, &mut m.function_type_parameters)?;
                m.function_handles.push(h);
            }
            TableKind::Signatures => m.signatures.push(signature(&mut c)?),
            TableKind::Identifiers => m.identifiers.push(identifier(&mut c)?),
            TableKind::ConstantPool => m.constants.push(constant(&mut c)?),
            TableKind::StructDefs => {
                let d = struct_def(&mut c, &mut m.fields)?;
                m.struct_defs.push(d);
            }
            TableKind::EnumDefs => {
                let d = enum_def(&mut c, &mut m.variants, &mut m.fields)?;
                m.enum_defs.push(d);
            }
            TableKind::FunctionInst => m.function_instantiations.push(instantiation(&mut c)?),
            TableKind::StructDefInst => m.struct_instantiations.push(instantiation(&mut c)?),
            TableKind::EnumDefInst => m.enum_instantiations.push(instantiation(&mut c)?),
            TableKind::FieldInst => m.field_instantiations.push(instantiation(&mut c)?),
            TableKind::FieldHandle => {
                let owner = c.index("struct definition")?;
                let field =
                    u16::try_from(c.uleb_max("field offset", FIELD_OFFSET_MAX)?).expect("small");
                m.field_handles.push((owner, field));
            }
            TableKind::VariantHandles => m.variant_handles.push(variant_handle(&mut c)?),
            TableKind::VariantInstHandles => {
                m.variant_instantiation_handles
                    .push(variant_handle(&mut c)?);
            }
            // Sui's binary config rejects extraneous bytes, which disallows metadata tables.
            TableKind::Metadata => return Err(Error::BadTableContents(kind)),
            TableKind::AddressIdentifiers | TableKind::FunctionDefs => {
                unreachable!("handled by the caller")
            }
        }
    }
    Ok(())
}

fn instantiation(c: &mut Cursor<'_>) -> Result<(u16, u16), Error> {
    Ok((c.index("definition or handle")?, c.index("signature")?))
}

fn variant_handle(c: &mut Cursor<'_>) -> Result<(u16, u16), Error> {
    let owner = c.index("variant handle owner")?;
    let tag = u16::try_from(c.uleb_max("variant tag", VARIANT_TAG_MAX_VALUE)?).expect("bounded");
    Ok((owner, tag))
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

fn arena_range(start: usize, end: usize) -> std::ops::Range<u32> {
    // Arenas hold at most 65535 × 255 entries, so u32 suffices.
    u32::try_from(start).expect("arena fits u32")..u32::try_from(end).expect("arena fits u32")
}

/// B-TP-1
fn datatype_handle(
    c: &mut Cursor<'_>,
    arena: &mut Vec<DatatypeTypeParameter>,
) -> Result<DatatypeHandle, Error> {
    let module = c.index("module handle")?;
    let name = c.index("identifier")?;
    let abilities = abilities(c)?;
    let count = c.uleb_max("type parameter count", TYPE_PARAMETER_COUNT_MAX)?;
    let start = arena.len();
    for _ in 0..count {
        let constraints = self::abilities(c)?;
        let is_phantom = c.uleb_max("phantom flag", 1)? != 0;
        arena.push(DatatypeTypeParameter {
            constraints,
            is_phantom,
        });
    }
    Ok(DatatypeHandle {
        module,
        name,
        abilities,
        type_parameters: arena_range(start, arena.len()),
    })
}

fn function_handle(
    c: &mut Cursor<'_>,
    arena: &mut Vec<AbilitySet>,
) -> Result<FunctionHandle, Error> {
    let module = c.index("module handle")?;
    let name = c.index("identifier")?;
    let parameters = c.index("signature")?;
    let return_ = c.index("signature")?;
    let count = c.uleb_max("type parameter count", TYPE_PARAMETER_COUNT_MAX)?;
    let start = arena.len();
    for _ in 0..count {
        arena.push(abilities(c)?);
    }
    Ok(FunctionHandle {
        module,
        name,
        parameters,
        return_,
        type_parameters: arena_range(start, arena.len()),
    })
}

/// B-ID-1: `[a-zA-Z][a-zA-Z0-9_]*` or `_[a-zA-Z0-9_]+`.
pub(crate) fn is_valid_identifier(b: &[u8]) -> bool {
    let tail_ok = || {
        b[1..]
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b'_')
    };
    match b.first() {
        Some(c) if c.is_ascii_alphabetic() => tail_ok(),
        Some(b'_') if b.len() > 1 => tail_ok(),
        _ => false,
    }
}

fn identifier<'a>(c: &mut Cursor<'a>) -> Result<&'a str, Error> {
    let len = usize::try_from(c.uleb_max("identifier size", IDENTIFIER_SIZE_MAX)?).expect("small");
    let bytes = c.take(len, "identifier")?;
    // Valid identifiers are ASCII, so validity implies UTF-8.
    if !is_valid_identifier(bytes) {
        return Err(Error::BadIdentifier(
            String::from_utf8_lossy(bytes).into_owned(),
        ));
    }
    Ok(std::str::from_utf8(bytes).expect("ASCII identifier"))
}

fn constant<'a>(c: &mut Cursor<'a>) -> Result<Constant<'a>, Error> {
    let type_ = read_token(c)?;
    let len = usize::try_from(c.uleb_max("constant size", CONSTANT_SIZE_MAX)?).expect("small");
    Ok(Constant {
        type_,
        data: c.take(len, "constant")?,
    })
}

fn signature<'a>(c: &mut Cursor<'a>) -> Result<Signature<'a>, Error> {
    let len = u8::try_from(c.uleb_max("signature size", SIGNATURE_SIZE_MAX)?).expect("bounded");
    let start = c.position();
    for _ in 0..len {
        read_token(c)?;
    }
    Ok(Signature {
        bytes: c.slice_from(start),
        len,
    })
}

fn fields<'a>(
    c: &mut Cursor<'a>,
    arena: &mut Vec<FieldDefinition<'a>>,
) -> Result<std::ops::Range<u32>, Error> {
    let count = c.uleb_max("field count", FIELD_COUNT_MAX)?;
    let start = arena.len();
    for _ in 0..count {
        arena.push(FieldDefinition {
            name: c.index("identifier")?,
            type_: read_token(c)?,
        });
    }
    Ok(arena_range(start, arena.len()))
}

/// B-DEF-1
fn struct_def<'a>(
    c: &mut Cursor<'a>,
    arena: &mut Vec<FieldDefinition<'a>>,
) -> Result<StructDefinition, Error> {
    let handle = c.index("datatype handle")?;
    let fields = match c.u8("struct flag")? {
        0x1 => None,
        0x2 => Some(fields(c, arena)?),
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
fn enum_def<'a>(
    c: &mut Cursor<'a>,
    variants: &mut Vec<VariantDefinition>,
    arena: &mut Vec<FieldDefinition<'a>>,
) -> Result<EnumDefinition, Error> {
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
    let start = variants.len();
    for _ in 0..count {
        let name = c.index("identifier")?;
        let fields = fields(c, arena)?;
        variants.push(VariantDefinition { name, fields });
    }
    Ok(EnumDefinition {
        handle,
        variants: arena_range(start, variants.len()),
    })
}

const FLAG_NATIVE: u8 = 0b10;
const FLAG_ENTRY: u8 = 0b100;

/// B-FN-1 (bytecode version ≥ 5 layout).
fn function_def(
    c: &mut Cursor<'_>,
    version: u32,
    checker: &CodeChecker<'_, '_>,
) -> Result<FunctionDefinition, Error> {
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
    if c.uleb_max("acquires count", ACQUIRES_COUNT_MAX)? != 0 {
        return Err(Error::GlobalStorage);
    }
    let is_native = flags & FLAG_NATIVE != 0;
    flags &= !FLAG_NATIVE;
    checker.function_handle(handle)?;
    if !is_native {
        code_unit(c, version, checker, handle)?;
    }
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
    })
}

/// B-CODE-1 + B-IDX-1: reads a code unit, bounds-checking every operand, keeping nothing.
fn code_unit(
    c: &mut Cursor<'_>,
    version: u32,
    checker: &CodeChecker<'_, '_>,
    handle: u16,
) -> Result<(), Error> {
    let locals = c.index("locals signature")?;
    let count = c.uleb_max("bytecode count", BYTECODE_COUNT_MAX)?;
    let mut code = checker.begin(handle, locals, count)?;
    for _ in 0..count {
        instruction(c, version, &mut code)?;
    }
    let mut tables = 0;
    if version >= VERSION_7 {
        tables = c.uleb_max("jump table count", JUMP_TABLE_INDEX_MAX)?;
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
            code.jump_table(head, branches)?;
            for _ in 0..branches {
                code.branch(c.uleb_max("bytecode index", BYTECODE_INDEX_MAX)?)?;
            }
        }
    }
    code.finish(tables)
}

fn instruction(
    c: &mut Cursor<'_>,
    version: u32,
    code: &mut bounds::Code<'_, '_>,
) -> Result<(), Error> {
    let enums = || {
        if version < VERSION_7 {
            Err(Error::EnumsBeforeV7)
        } else {
            Ok(())
        }
    };
    match c.u8("opcode")? {
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
        | 0x4B..=0x4D => {}
        0x03..=0x05 => code.branch(c.uleb_max("bytecode index", BYTECODE_INDEX_MAX)?)?,
        0x06 => {
            c.take(8, "u64")?;
        }
        0x07 => code.constant(c.index("constant")?)?,
        0x0A..=0x0E => code.local(c.uleb_max("local index", LOCAL_INDEX_MAX)?)?,
        0x0F | 0x10 => code.field_handle(c.index("field handle")?)?,
        0x36 | 0x37 => code.field_instantiation(c.index("field instantiation")?)?,
        0x11 => code.function_handle(c.index("function handle")?)?,
        0x38 => code.function_instantiation(c.index("function instantiation")?)?,
        0x12 | 0x13 => code.struct_definition(c.index("struct definition")?)?,
        0x39 | 0x3A => code.struct_instantiation(c.index("struct instantiation")?)?,
        0x31 => {
            c.u8("u8")?;
        }
        0x32 => {
            c.take(16, "u128")?;
        }
        // VEC_PACK / VEC_UNPACK: signature + u64 count.
        0x40 | 0x46 => {
            code.vector_signature(c.index("signature")?)?;
            c.take(8, "u64")?;
        }
        0x41..=0x45 | 0x47 => code.vector_signature(c.index("signature")?)?,
        0x48 => {
            c.take(2, "u16")?;
        }
        0x49 => {
            c.take(4, "u32")?;
        }
        0x4A => {
            c.take(32, "u256")?;
        }
        0x4E | 0x50..=0x52 => {
            enums()?;
            code.variant_handle(c.uleb_max("variant handle", VARIANT_HANDLE_INDEX_MAX)?)?;
        }
        0x4F | 0x53..=0x55 => {
            enums()?;
            let i = c.uleb_max("variant instantiation handle", VARIANT_HANDLE_INDEX_MAX)?;
            code.variant_instantiation(i)?;
        }
        0x56 => {
            enums()?;
            code.jump_table_index(c.uleb_max("jump table index", JUMP_TABLE_INDEX_MAX)?);
        }
        // Deprecated global storage.
        0x29..=0x2D | 0x3B..=0x3F => return Err(Error::GlobalStorage),
        other => return Err(Error::UnknownOpcode(other)),
    }
    Ok(())
}
