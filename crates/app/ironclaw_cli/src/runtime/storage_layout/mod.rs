//! Boot-time admission and one-shot migration for the profile-stable Reborn
//! durable layout.
//!
//! This is deliberately a single bounded transition for this one filesystem
//! layout. It does not discover arbitrary roots, infer workspace owners, or
//! serve as a generic migration framework.

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read as _, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow, bail};
use ironclaw_config::{
    DurableStateKind, LayoutManifest, LayoutRequirement, LegacyStorageSource,
    ProfileTransitionAdmission, RebornHome, RebornStoragePaths,
};
use ironclaw_host_api::ids::{TenantId, UserId};
use serde::{Deserialize, Serialize};

mod admission;
mod filesystem;
mod locks;
mod model;
mod mover;

pub(crate) use admission::{
    admit_startup_layout, ensure_ready_layout, inspect_ready_layout,
    ready_legacy_skill_snapshot_source, ready_memory_provider_app_id,
};
pub(crate) use model::{StartupLayoutAdmission, StorageMigrationPolicy};
pub(crate) use mover::migrate_legacy_layout;

#[cfg(test)]
mod tests;
