//! Frozen, self-contained port of the v1 (`ironclaw_legacy`) read path this
//! crate needs, so migration keeps working once `src/` is deleted under Tier
//! B (`docs/plans/2026-07-02-reborn-internal-module-refactor.md` §8). Mirrors
//! the pattern [`crate::v2_model`] already established for the deleted
//! engine-v2 types: freeze the on-disk contract, stop depending on the live
//! crate that used to own it.
//!
//! Scope: only what `convert::*` actually calls — 7 `Database` trait methods
//! (`queries`), the v1 secrets store's list/get/decrypt (`secrets`), and the
//! installed wasm tool/channel stores' list/get_capabilities (`wasm_stores`).
//! Not a general-purpose v1 client.

pub(crate) mod connect;
pub(crate) mod error;
pub(crate) mod libsql_helpers;
#[cfg(test)]
mod postgres_tests;
pub(crate) mod queries;
pub(crate) mod secrets;
pub(crate) mod types;
pub(crate) mod wasm_stores;

pub(crate) use connect::{LegacyDb, LegacyHandles, connect};
pub(crate) use types::{Routine, RoutineAction, Trigger, UserIdentityRecord};
