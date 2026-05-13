//! Content-addressed identity for hooks.
//!
//! Every active hook has a stable, version-pinned identity. The `HookId` is a
//! blake3 digest derived from `(extension_id, hook_local_id, hook_version,
//! extension_version)` so that replay across version drift refuses silently:
//! a checkpoint persisted under one `HookId` will not collide with the same
//! `(extension_id, hook_local_id)` shipped under a different version.
//!
//! # Cross-crate wire format
//!
//! `HookId::to_hex()` produces a 64-character lowercase ASCII hex string and
//! that exact format is part of the **cross-crate contract**. It is what the
//! dispatcher emits into `LoopHostMilestoneKind::HookDispatched { hook_id, .. }`
//! and `HookDecisionEmitted { hook_id, .. }` / `HookFailed { hook_id, .. }` in
//! `ironclaw_turns`, and what downstream SSE / audit / replay consumers parse
//! and key on. Changing the encoding (e.g. switching to base32, adding a
//! prefix, uppercasing) is a wire-format break and **requires bumping a
//! contract version** so consumers can migrate. The pinning tests
//! `hook_id_hex_format_is_stable_64_lowercase_chars` (in this module) and
//! `hook_id_string_serialization_matches_to_hex` (in `telemetry::tests`) are
//! the regression guards for that invariant.

use std::fmt;

use serde::{Deserialize, Serialize};

/// 32-byte blake3 digest identifying a hook.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HookId(pub(crate) [u8; 32]);

impl HookId {
    /// Derive a content-addressed id. All four fields are length-prefixed when
    /// fed to the hasher to prevent canonicalization collisions across fields.
    pub fn derive(
        extension: &ExtensionId,
        extension_version: &str,
        local: &HookLocalId,
        hook_version: HookVersion,
    ) -> Self {
        let mut hasher = blake3::Hasher::new();
        feed_field(&mut hasher, extension.0.as_bytes());
        feed_field(&mut hasher, extension_version.as_bytes());
        feed_field(&mut hasher, local.0.as_bytes());
        feed_field(&mut hasher, &hook_version.0.to_le_bytes());
        Self(hasher.finalize().into())
    }

    /// For Builtin hooks whose identity is a stable canonical path + symbol.
    pub fn for_builtin(canonical_path: &str, hook_version: HookVersion) -> Self {
        let mut hasher = blake3::Hasher::new();
        feed_field(&mut hasher, b"builtin");
        feed_field(&mut hasher, canonical_path.as_bytes());
        feed_field(&mut hasher, &hook_version.0.to_le_bytes());
        Self(hasher.finalize().into())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for byte in self.0 {
            s.push_str(&format!("{byte:02x}"));
        }
        s
    }
}

impl fmt::Debug for HookId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Display only the first 8 bytes for log readability; full hex via
        // to_hex(). Avoids dumping 64-char strings into trace logs.
        let mut head = String::with_capacity(16);
        for byte in self.0.iter().take(4) {
            head.push_str(&format!("{byte:02x}"));
        }
        write!(f, "HookId({head}…)")
    }
}

impl fmt::Display for HookId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Monotonic per-hook version. Bumped explicitly by the hook author at
/// registration time when the hook's behavior changes; replay across a version
/// bump refuses to silently re-evaluate.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct HookVersion(pub u64);

impl HookVersion {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(1);
}

impl fmt::Display for HookVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

/// Identifier of the extension that supplied a hook (for `Installed`-tier
/// hooks). Builtin hooks do not carry an `ExtensionId`.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct ExtensionId(pub String);

impl fmt::Display for ExtensionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Extension-author-chosen identifier for the hook within their manifest.
/// Combined with `ExtensionId` and versions to form a globally-unique `HookId`.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct HookLocalId(pub String);

impl fmt::Display for HookLocalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn feed_field(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_is_deterministic() {
        let a = HookId::derive(
            &ExtensionId("polymarket-trader".to_string()),
            "0.4.2",
            &HookLocalId("daily-order-cap".to_string()),
            HookVersion::ONE,
        );
        let b = HookId::derive(
            &ExtensionId("polymarket-trader".to_string()),
            "0.4.2",
            &HookLocalId("daily-order-cap".to_string()),
            HookVersion::ONE,
        );
        assert_eq!(a, b);
    }

    #[test]
    fn version_bump_changes_id() {
        let a = HookId::derive(
            &ExtensionId("ext".to_string()),
            "1.0",
            &HookLocalId("h".to_string()),
            HookVersion(1),
        );
        let b = HookId::derive(
            &ExtensionId("ext".to_string()),
            "1.0",
            &HookLocalId("h".to_string()),
            HookVersion(2),
        );
        assert_ne!(a, b);
    }

    #[test]
    fn extension_version_bump_changes_id() {
        let a = HookId::derive(
            &ExtensionId("ext".to_string()),
            "1.0",
            &HookLocalId("h".to_string()),
            HookVersion::ONE,
        );
        let b = HookId::derive(
            &ExtensionId("ext".to_string()),
            "1.1",
            &HookLocalId("h".to_string()),
            HookVersion::ONE,
        );
        assert_ne!(a, b);
    }

    #[test]
    fn length_prefix_prevents_field_concatenation_collision() {
        // Without length-prefixing, ("ab", "c") and ("a", "bc") would collide.
        // Length-prefixing must keep them distinct.
        let a = HookId::derive(
            &ExtensionId("ab".to_string()),
            "1.0",
            &HookLocalId("c".to_string()),
            HookVersion::ONE,
        );
        let b = HookId::derive(
            &ExtensionId("a".to_string()),
            "1.0",
            &HookLocalId("bc".to_string()),
            HookVersion::ONE,
        );
        assert_ne!(a, b);
    }

    #[test]
    fn builtin_id_distinct_from_extension_id() {
        let installed = HookId::derive(
            &ExtensionId("builtin".to_string()),
            "x",
            &HookLocalId("path::module".to_string()),
            HookVersion::ONE,
        );
        let builtin = HookId::for_builtin("path::module", HookVersion::ONE);
        assert_ne!(installed, builtin);
    }

    /// The hex format produced by `HookId::to_hex()` is part of the
    /// cross-crate contract: it is what the dispatcher serializes into
    /// `LoopHostMilestoneKind::Hook*` variants in `ironclaw_turns`, and what
    /// downstream SSE / audit / replay consumers key on. This test pins the
    /// format — any change here is a wire-format break and must be
    /// accompanied by a contract version bump and consumer migration.
    #[test]
    fn hook_id_hex_format_is_stable_64_lowercase_chars() {
        let id = HookId::for_builtin("crate::safety::policy", HookVersion::ONE);
        let hex = id.to_hex();
        assert_eq!(hex.len(), 64, "blake3 hex must be exactly 64 chars");
        assert!(
            hex.chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "hex must be ASCII lowercase 0-9a-f, got {hex}"
        );
        // Also exercise the derive path to ensure no per-constructor drift.
        let derived = HookId::derive(
            &ExtensionId("ext".to_string()),
            "1.0",
            &HookLocalId("h".to_string()),
            HookVersion::ONE,
        );
        let derived_hex = derived.to_hex();
        assert_eq!(derived_hex.len(), 64);
        assert!(
            derived_hex
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        );
    }

    #[test]
    fn debug_format_is_truncated() {
        let id = HookId::for_builtin("crate::safety::policy", HookVersion::ONE);
        let debug = format!("{id:?}");
        assert!(debug.starts_with("HookId("));
        assert!(debug.ends_with("…)"));
        assert!(debug.len() < 24, "debug should be short, got {debug}");
    }
}
