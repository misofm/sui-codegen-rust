//! Generated datatypes, end to end: BCS layout, struct tags, strict decoding (docs/SPEC.md §T).

use movegen_e2e::generated::{record, record_shop, sui};
use movegen_runtime::{Deployment, Error, MoveStruct as _, decode};
use sui_sdk_types::{Address, StructTag, TypeTag};

fn mainnet() -> Deployment {
    Deployment::from_lock_json(include_str!("../deployments/mainnet.json")).unwrap()
}

fn usdc() -> TypeTag {
    "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC"
        .parse()
        .unwrap()
}

// spec: T-ENUM-1 (variant tag = declaration order, not alphabetical)
#[test]
fn enum_layout() {
    use record_shop::listing::Pricing;
    let fixed = bcs::to_bytes(&Pricing::Fixed(5)).unwrap();
    let floor = bcs::to_bytes(&Pricing::Floor(5)).unwrap();
    assert_eq!(fixed[0], 0);
    assert_eq!(floor[0], 1);
    assert_eq!(&fixed[1..], &5u64.to_le_bytes());
    assert_eq!(
        bcs::from_bytes::<Pricing>(&floor).unwrap(),
        Pricing::Floor(5)
    );
}

// spec: T-TAG-1 (tags use the defining id; phantom arguments travel as tags)
#[test]
fn struct_tags() {
    let d = mainnet();
    let tag = record_shop::listing::ListingKey::struct_tag(&d, vec![usdc()]).unwrap();
    assert_eq!(
        *tag.address(),
        d.defining_id("record_shop", "listing", "ListingKey")
            .unwrap()
    );
    assert_eq!(
        tag.to_string(),
        format!("{}::listing::ListingKey<{}>", tag.address(), usdc())
    );
    assert!(record_shop::listing::ListingKey::is(&d, &tag).unwrap());
    assert_eq!(
        record::pressing::PressingKey::struct_tag(&d, vec![usdc()]).unwrap_err(),
        Error::TypeArguments {
            expected: 0,
            got: 1
        }
    );
    // Empty structs carry Move's `dummy_field`; positional structs are tuples.
    assert_eq!(
        bcs::to_bytes(&record_shop::listing::ListingKey { dummy_field: false }).unwrap(),
        [0]
    );
    assert_eq!(
        bcs::to_bytes(&record::pressing::PressingKey(3)).unwrap(),
        [3, 0]
    );
}

// spec: T-DEC-1 (exact type, every byte consumed)
#[test]
fn strict_decoding() {
    let d = mainnet();
    let clock = sui::clock::Clock {
        id: movegen_runtime::Uid(Address::new([6; 32])),
        timestamp_ms: 42,
    };
    let bytes = bcs::to_bytes(&clock).unwrap();
    let tag = sui::clock::Clock::struct_tag(&d, vec![]).unwrap();
    let decoded = decode::<sui::clock::Clock>(&d, &tag, &bytes).unwrap();
    assert_eq!(decoded.value, clock);
    assert!(decoded.type_arguments.is_empty());

    let other: StructTag = record::pressing::PressingKey::struct_tag(&d, vec![]).unwrap();
    assert!(matches!(
        decode::<sui::clock::Clock>(&d, &other, &bytes),
        Err(Error::WrongType { .. })
    ));
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(matches!(
        decode::<sui::clock::Clock>(&d, &tag, &trailing),
        Err(Error::Decode(_))
    ));
}
