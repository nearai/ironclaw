//! Predicate bucket-identity hashing for the Postgres backend.
//!
//! The derivation itself lives in the shared crate
//! ([`ironclaw_hooks::predicate_hash`]) so the durable DB identity contract has
//! a single source of truth across backends (it previously diverged: this
//! crate used a 4-byte big-endian `u32` length prefix while libSQL used 8-byte
//! little-endian `u64`). This module is now a thin alias so Postgres call sites
//! keep their local names; see the canonical module for the length-prefix /
//! map-discriminant rationale. Digests are stored as `BYTEA`.

pub(crate) use ironclaw_hooks::predicate_hash::{
    Digest, invocation_key_hash, scope_hash, value_key_hash,
};
