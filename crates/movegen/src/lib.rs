//! `movegen`: first-party Rust bindings for Sui Move packages.
//!
//! Pipeline: `movegen.toml` → [`refresh`] (network, via a [`Source`]) → the committed
//! [`Lock`] → generation (offline). See `docs/PLAN.md` and the rules in `docs/SPEC.md`.

pub mod config;
mod error;
mod generate;
pub mod lock;
pub mod refresh;

pub use config::Config;
pub use error::Error;
pub use generate::{Change, Generated, generate};
pub use lock::Lock;
pub use refresh::{ChainInfo, FetchedPackage, Source, compare_networks, refresh};
