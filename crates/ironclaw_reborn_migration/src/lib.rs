//! v1 / engine-v2 → Reborn state migration.
//!
//! Reads a legacy IronClaw v1 database (PostgreSQL or libSQL) — which is also
//! where engine-v2 state lives, as JSON blobs in `memory_documents` — and
//! writes the equivalent Reborn state into the `RootFilesystem` KV substrate
//! plus the triggers database. Threads and automations (routines + engine-v2
//! missions) convert without loss; anything that has no Reborn representation
//! today is recorded in a [`MigrationReport`] rather than silently dropped.
//!
//! The crate is a library (this module) plus a thin binary (`src/main.rs`) so
//! the conversion engine can later be reused inside `ironclaw-reborn` startup.

pub mod error;
pub mod options;
pub mod report;

mod target;

mod extension_ownership;

#[cfg(feature = "full-migration")]
mod convert;
#[cfg(feature = "full-migration")]
mod mounts;
#[cfg(feature = "full-migration")]
mod source;
#[cfg(feature = "full-migration")]
mod v2_model;

pub use extension_ownership::{
    ExtensionOwnershipMigrationOptions, ExtensionOwnershipMigrationReport,
    run_extension_ownership_migration,
};

pub use error::MigrationError;
pub use options::{MigrationOptions, SourceDb, TargetStore};
pub use report::{Domain, LossReason, LossyItem, MigrationReport, MigrationStats};

/// Run a full migration: open the v1 source and Reborn target, convert every
/// in-scope domain, and return the outcome report.
///
/// Infrastructure failures (cannot open a store, cannot write a record) abort
/// with a [`MigrationError`]. Per-item representation gaps do not abort — they
/// are accumulated as [`LossyItem`]s on the returned report.
#[cfg(feature = "full-migration")]
pub async fn run_migration(options: MigrationOptions) -> Result<MigrationReport, MigrationError> {
    let mut report = MigrationReport::new(options.dry_run);

    let src = source::V1Source::open(&options.source).await?;
    let mut tgt = target::RebornTarget::open(&options).await?;

    convert::threads::run(&src, &mut tgt, &options, &mut report).await?;
    convert::automations::run(&src, &mut tgt, &options, &mut report).await?;
    convert::memory::run(&src, &mut tgt, &options, &mut report).await?;
    convert::jobs::run(&src, &mut tgt, &options, &mut report).await?;
    convert::secrets::run(&src, &mut tgt, &options, &mut report).await?;
    convert::extensions::run(&src, &mut tgt, &options, &mut report).await?;
    convert::identities::run(&src, &mut tgt, &options, &mut report).await?;
    convert::heartbeat::run(&src, &mut tgt, &options, &mut report).await?;
    convert::settings::run(&src, &mut tgt, &options, &mut report).await?;

    Ok(report)
}
