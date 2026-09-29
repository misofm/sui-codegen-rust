//! Type identity, strict decoding, and the framework types every binding shares (T rules).
//!
//! Phantom type parameters are not part of Rust types: they never affect BCS layout, and Miso's
//! type arguments (currencies) are runtime values anyway. They travel as `TypeTag`s instead —
//! in [`Typed`], in struct tags, and in call targets.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sui_sdk_types::{Address, Identifier, StructTag, TypeTag};

use crate::call::{Key, Pure};
use crate::{Deployment, Error};

/// A Move struct or enum with a generated (or built-in) binding.
pub trait MoveStruct {
    /// Package symbol in the lock (`sui`, `record_shop`, …).
    const PACKAGE: &'static str;
    /// Module name.
    const MODULE: &'static str;
    /// Datatype name.
    const NAME: &'static str;
    /// Number of type parameters (phantom included).
    const TYPE_PARAMETERS: usize;

    /// The struct tag for these type arguments, addressed by the type's defining id (T-TAG-1).
    fn struct_tag(d: &Deployment, type_arguments: Vec<TypeTag>) -> Result<StructTag, Error> {
        if type_arguments.len() != Self::TYPE_PARAMETERS {
            return Err(Error::TypeArguments {
                expected: Self::TYPE_PARAMETERS,
                got: type_arguments.len(),
            });
        }
        Ok(StructTag::new(
            d.defining_id(Self::PACKAGE, Self::MODULE, Self::NAME)?,
            Identifier::from_static(Self::MODULE),
            Identifier::from_static(Self::NAME),
            type_arguments,
        ))
    }

    /// Whether `tag` is this type (any type arguments of the right arity).
    fn is(d: &Deployment, tag: &StructTag) -> Result<bool, Error> {
        Ok(
            *tag.address() == d.defining_id(Self::PACKAGE, Self::MODULE, Self::NAME)?
                && tag.module().as_str() == Self::MODULE
                && tag.name().as_str() == Self::NAME
                && tag.type_params().len() == Self::TYPE_PARAMETERS,
        )
    }
}

/// A decoded value with the type arguments of its tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Typed<T> {
    /// Type arguments from the struct tag.
    pub type_arguments: Vec<TypeTag>,
    /// The value.
    pub value: T,
}

/// Decodes BCS known to have type `tag` (an object's or event's type), strictly: the tag must be
/// exactly `T` (defining id, module, name, arity) and every byte must be consumed (T-DEC-1).
pub fn decode<T: MoveStruct + DeserializeOwned>(
    d: &Deployment,
    tag: &StructTag,
    bcs: &[u8],
) -> Result<Typed<T>, Error> {
    if !T::is(d, tag)? {
        return Err(Error::WrongType {
            expected: format!("{}::{}::{}", T::PACKAGE, T::MODULE, T::NAME),
            found: tag.to_string(),
        });
    }
    let value = bcs::from_bytes(bcs).map_err(|e| Error::Decode(e.to_string()))?;
    Ok(Typed {
        type_arguments: tag.type_params().to_vec(),
        value,
    })
}

macro_rules! builtin {
    ($ty:ty, $package:literal, $module:literal, $name:literal, $arity:literal) => {
        impl MoveStruct for $ty {
            const PACKAGE: &'static str = $package;
            const MODULE: &'static str = $module;
            const NAME: &'static str = $name;
            const TYPE_PARAMETERS: usize = $arity;
        }
    };
}

/// `0x2::object::UID`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Uid(pub Address);
builtin!(Uid, "sui", "object", "UID", 0);

/// `0x2::object::ID` (a pure type).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Id(pub Address);
builtin!(Id, "sui", "object", "ID", 0);
impl Pure for Id {}

/// `0x1::ascii::String` (a pure type).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AsciiString(pub String);
builtin!(AsciiString, "std", "ascii", "String", 0);
impl Pure for AsciiString {}

/// `0x1::type_name::TypeName` (not pure).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TypeName {
    /// Fully qualified type name, without `0x`.
    pub name: AsciiString,
}
builtin!(TypeName, "std", "type_name", "TypeName", 0);

/// `0x2::url::Url` (not pure).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Url {
    /// The URL.
    pub url: AsciiString,
}
builtin!(Url, "sui", "url", "Url", 0);

/// `0x2::balance::Balance<phantom T>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Balance {
    /// Amount.
    pub value: u64,
}
builtin!(Balance, "sui", "balance", "Balance", 1);

/// `0x2::balance::Supply<phantom T>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Supply {
    /// Total supply.
    pub value: u64,
}
builtin!(Supply, "sui", "balance", "Supply", 1);

/// `0x2::coin::Coin<phantom T>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Coin {
    /// Object id.
    pub id: Uid,
    /// Balance.
    pub balance: Balance,
}
builtin!(Coin, "sui", "coin", "Coin", 1);
impl Key for Coin {}

/// `0x2::transfer::Receiving<phantom T>` (the value; pass a [`crate::ReceivingRef`] to create one).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Receiving {
    /// Object to receive.
    pub id: Id,
    /// Its version.
    pub version: u64,
}
builtin!(Receiving, "sui", "transfer", "Receiving", 1);

/// `0x2::vec_map::Entry<K, V>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VecMapEntry<K, V> {
    /// Key.
    pub key: K,
    /// Value.
    pub value: V,
}

/// `0x2::vec_map::VecMap<K, V>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VecMap<K, V> {
    /// Entries in insertion order.
    pub contents: Vec<VecMapEntry<K, V>>,
}

/// `0x2::vec_set::VecSet<K>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VecSet<K> {
    /// Elements in insertion order.
    pub contents: Vec<K>,
}

/// `0x2::table::Table<phantom K, phantom V>` (and `bag::Bag`, `object_table::ObjectTable`,
/// `object_bag::ObjectBag`): the header; entries are dynamic fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Table {
    /// Object id (parent of the entries).
    pub id: Uid,
    /// Number of entries.
    pub size: u64,
}
builtin!(Table, "sui", "table", "Table", 2);

/// `0x2::linked_table::LinkedTable<K, phantom V>`: the header.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LinkedTable<K> {
    /// Object id.
    pub id: Uid,
    /// Number of entries.
    pub size: u64,
    /// First key.
    pub head: Option<K>,
    /// Last key.
    pub tail: Option<K>,
}

/// `0x2::table_vec::TableVec<phantom T>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TableVec {
    /// The backing table.
    pub contents: Table,
}
