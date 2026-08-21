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

pub(crate) use error::ReleasePairMigrationError;
pub(crate) use rc1_to_1_1::{
    Rc1To11ChannelStateMigrationOutcome, Rc1To11ExtensionReports, Rc1To11Migration,
    Rc1To11MigrationInput, migrate_rc1_hosted_extension_snapshots,
};
pub(crate) use workspace::LegacyWorkspaceMigrationInput;
