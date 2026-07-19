//! Reborn operator migration utilities.
//!
//! The legacy v1 state importer retired with the root `ironclaw` runtime. This
//! crate now ships Reborn-only operator migrations that work directly against
//! the Reborn state substrate.

pub mod error;
pub mod options;

mod target;

mod extension_ownership;

pub use extension_ownership::{
    ExtensionOwnershipMigrationOptions, ExtensionOwnershipMigrationReport,
    run_extension_ownership_migration,
};

pub use error::MigrationError;
pub use options::TargetStore;
