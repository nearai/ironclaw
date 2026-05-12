//! Boot configuration contracts for the standalone IronClaw Reborn binary.
//!
//! This crate is intentionally small and has no IronClaw workspace dependencies.
//! It owns process/environment boot configuration that must be shared by the
//! `ironclaw-reborn` binary and later Reborn runtime composition without pulling
//! in the v1 root application.

pub mod boot;
pub mod doctor;
pub mod home;
pub mod profile;

pub use boot::RebornBootConfig;
pub use doctor::RebornDoctorReport;
pub use home::{REBORN_HOME_ENV, RebornConfigError, RebornHome, RebornHomeSource};
pub use profile::{REBORN_PROFILE_ENV, RebornProfile};
