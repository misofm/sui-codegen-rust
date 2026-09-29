//! A strict, dependency-light reader for Sui Move bytecode (format v6–v7).
//!
//! It reads exactly the tables that describe a module's ABI — handles, signatures, identifiers,
//! addresses, constants, struct/enum/function definitions, friends — and validates-then-skips
//! code. The behaviour mirrors `move-binary-format`'s deserializer at Sui `mainnet-v1.80.1`; each
//! check cites a rule in `docs/SPEC.md`, and the `oracle/` crate differentially tests the whole
//! corpus against Mysten's implementation.
//!
//! ```no_run
//! # fn f(bytes: &[u8]) -> Result<(), movegen_bytecode::Error> {
//! let package = movegen_bytecode::Package::from_bcs(bytes)?;
//! for module in package.module_abis()? {
//!     println!("{}::{} ({} functions)", module.address, module.name, module.functions.len());
//! }
//! # Ok(()) }
//! ```

pub mod abi;
mod bounds;
mod cursor;
mod deserialize;
mod error;
pub mod model;
mod package;
mod sig;

pub use deserialize::deserialize;
pub use error::{Error, TableKind};
pub use model::CompiledModule;
pub use package::{Linkage, Package, TypeOrigin};
