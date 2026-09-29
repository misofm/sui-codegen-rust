//! Runtime for `movegen`-generated Sui Move bindings.
//!
//! Small and wasm-clean: `sui-sdk-types`, `sui-transaction-builder` (without its gRPC
//! features), `serde`, `bcs`. Generated code refers to it by absolute path
//! (`::movegen_runtime::…`) and never contains a network-specific id.

mod call;
mod deployment;
mod types;

pub use call::{
    Any, Arg, ArgKind, CallError, Input, IntoInput, Key, MoveTarget, ObjectRef, Pure, Receive,
    ReceivingRef, input, make_move_vec, nested, system_object, type_arguments,
};
pub use deployment::{Deployment, PackageIds, SystemObject};
pub use types::{
    AsciiString, Balance, Coin, Id, LinkedTable, MoveStruct, Receiving, Supply, Table, TableVec,
    TypeName, Typed, Uid, Url, VecMap, VecMapEntry, VecSet, decode,
};

/// Re-exports generated code relies on.
#[doc(hidden)]
pub mod __private {
    pub use serde;
    pub use sui_sdk_types;
    pub use sui_transaction_builder;
}

/// Runtime errors.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The deployment lacks a package.
    #[error("deployment has no package {0:?}")]
    MissingPackage(&'static str),
    /// The deployment lacks a system object.
    #[error("deployment has no system object {0:?}")]
    MissingSystemObject(&'static str),
    /// The deployment data is malformed.
    #[error("invalid deployment: {0}")]
    Deployment(String),
    /// A tag is not the expected type.
    #[error("expected {expected}, found {found}")]
    WrongType {
        /// Expected type.
        expected: String,
        /// Tag found.
        found: String,
    },
    /// Wrong number of type arguments.
    #[error("expected {expected} type arguments, got {got}")]
    TypeArguments {
        /// Expected count.
        expected: usize,
        /// Supplied count.
        got: usize,
    },
    /// BCS did not decode (or had trailing bytes).
    #[error("decode: {0}")]
    Decode(String),
}
