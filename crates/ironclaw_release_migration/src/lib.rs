//! Versioned startup migrations between supported IronClaw releases.
//!
//! The lifecycle machinery is release-neutral. Each concrete release pair
//! remains an explicit plan so its compatibility readers can be removed when
//! that direct upgrade path reaches end of support.

#![warn(unreachable_pub)]

mod error;
mod lifecycle;
mod rc1_to_1_1;
mod workspace;

pub use error::ReleasePairMigrationError;
pub use rc1_to_1_1::{
    ChannelRootMigrationReport, ChannelScopeMigrationReport, ExtensionInstallationMigrationReport,
    Rc1To11ExtensionReports, Rc1To11Migration, Rc1To11MigrationInput,
    discover_rc1_hosted_extension_snapshots, migrate_channel_roots,
    migrate_rc1_hosted_extension_snapshots, validate_channel_thread_references,
};
pub use workspace::{
    LegacyWorkspaceMigrationInput, LegacyWorkspaceMigrationReport,
    migrate_legacy_workspace_snapshot,
};
