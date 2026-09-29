//! Rule-by-rule tests (docs/SPEC.md §B), each a targeted corruption of a real module:
//! `0x1::hash` from the mainnet framework.

use movegen_bytecode::{Error, Package, abi::Module, deserialize};

fn hash_module() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/packages/mainnet/0x0000000000000000000000000000000000000000000000000000000000000001.bcs"
    );
    let package = Package::from_bcs(&std::fs::read(path).unwrap()).unwrap();
    package.modules["hash"].clone()
}

/// Byte positions of each table header's kind byte.
fn header_kind_positions(m: &[u8]) -> Vec<usize> {
    fn skip_uleb(m: &[u8], mut at: usize) -> usize {
        while m[at] & 0x80 != 0 {
            at += 1;
        }
        at + 1
    }
    let count = usize::from(m[8]);
    assert!(count < 0x80);
    let mut at = 9;
    let mut out = Vec::new();
    for _ in 0..count {
        out.push(at);
        at = skip_uleb(m, skip_uleb(m, at + 1));
    }
    out
}

// spec: B-HDR-1
#[test]
fn rejects_bad_magic() {
    let mut m = hash_module();
    m[0..4].copy_from_slice(&[0xDE, 0xAD, 0xC0, 0xDE]);
    assert_eq!(deserialize(&m), Err(Error::BadMagic));
}

// spec: B-HDR-2
#[test]
fn version_and_flavor() {
    let original = hash_module();
    assert_eq!(
        u32::from_le_bytes(original[4..8].try_into().unwrap()),
        0x0500_0007
    );
    for (version, ok) in [
        (0x0500_0007u32, true),
        (0x0500_0006, true),
        (6, true),
        (7, false),
        (0x0400_0007, false),
        (5, false),
        (0x0500_0008, false),
    ] {
        let mut m = original.clone();
        m[4..8].copy_from_slice(&version.to_le_bytes());
        let result = deserialize(&m);
        if ok {
            // v6 rejects the enum/u16+ features only if present; hash has neither.
            assert!(result.is_ok(), "{version:#x}: {result:?}");
        } else {
            assert!(
                matches!(result, Err(Error::UnsupportedVersion { .. })),
                "{version:#x}: {result:?}"
            );
        }
    }
}

// spec: B-END-1
#[test]
fn rejects_trailing_bytes() {
    let mut m = hash_module();
    m.push(0);
    assert_eq!(deserialize(&m), Err(Error::TrailingBytes(1)));
}

// spec: B-TBL-1, B-TBL-2
#[test]
fn rejects_bad_table_headers() {
    let original = hash_module();
    // Header layout: magic(4) version(4) count(uleb) then [kind, offset, size]...
    let first_kind = 9;
    let mut unknown = original.clone();
    unknown[first_kind] = 0x09; // 0x9 is not a table kind
    assert_eq!(deserialize(&unknown), Err(Error::UnknownTable(0x09)));

    // Duplicate: give the second table header the first header's kind.
    let mut dup = original.clone();
    let kinds = header_kind_positions(&original);
    dup[kinds[1]] = dup[kinds[0]];
    assert_eq!(
        deserialize(&dup),
        Err(Error::BadTableHeader("duplicate table"))
    );

    // Gap: shift the second table's offset by one.
    let mut gap = original.clone();
    gap[kinds[1] + 1] += 1;
    assert_eq!(
        deserialize(&gap),
        Err(Error::BadTableHeader("tables are not contiguous"))
    );
}

// spec: B-ID-1
#[test]
fn identifier_rules() {
    for (s, ok) in [
        ("hash", true),
        ("_x", true),
        ("A1_b", true),
        ("_", false),
        ("1a", false),
        ("a-b", false),
        ("", false),
        ("é", false),
    ] {
        let valid = s
            .as_bytes()
            .first()
            .is_some_and(|c| c.is_ascii_alphabetic() || (*c == b'_' && s.len() > 1))
            && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_');
        assert_eq!(valid, ok, "{s:?}");
    }
    // A real module's identifier corrupted to a digit-leading name is rejected.
    let mut m = hash_module();
    let at = m.windows(8).position(|w| w == b"sha2_256").unwrap();
    m[at] = b'2';
    assert!(
        matches!(deserialize(&m), Err(Error::BadIdentifier(_))),
        "{:?}",
        deserialize(&m)
    );
}

// spec: B-FN-1
#[test]
fn native_public_functions() {
    let module = Module::from_bytes(&hash_module()).unwrap();
    let names: Vec<_> = module
        .functions
        .iter()
        .map(|f| (f.name.as_str(), f.native))
        .collect();
    assert_eq!(names, [("sha2_256", true), ("sha3_256", true)]);
}
