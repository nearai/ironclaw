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
use thiserror::Error;

/// Maximum byte length of an identity string segment. Mirrors the
/// `validate_name_segment` limit in `ironclaw_host_api::ids` so the two
/// `ExtensionId` types share the same envelope.
pub const MAX_IDENTITY_BYTES: usize = 128;

/// Validation failures for identity newtypes. Construction sites convert these
/// to richer errors at their layer; the variants here are deliberately small
/// and side-effect-free so that any caller can reuse the same checks.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InvalidIdentity {
    #[error("{kind} id must not be empty")]
    Empty { kind: &'static str },
    #[error("{kind} id `{value}` exceeds {max} bytes")]
    TooLong {
        kind: &'static str,
        value: String,
        max: usize,
    },
    #[error("{kind} id `{value}` must start with a lowercase ASCII letter or digit")]
    BadLeadingChar { kind: &'static str, value: String },
    #[error(
        "{kind} id `{value}` may only contain lowercase ASCII letters, digits, '_', '-', and '.'"
    )]
    BadChar { kind: &'static str, value: String },
    #[error("{kind} id `{value}` may not contain '..' or empty dot segments")]
    BadDotSegment { kind: &'static str, value: String },
}

fn validate_identity_segment(kind: &'static str, value: &str) -> Result<(), InvalidIdentity> {
    if value.is_empty() {
        return Err(InvalidIdentity::Empty { kind });
    }
    if value.len() > MAX_IDENTITY_BYTES {
        return Err(InvalidIdentity::TooLong {
            kind,
            value: value.to_string(),
            max: MAX_IDENTITY_BYTES,
        });
    }
    let first = value.as_bytes()[0];
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return Err(InvalidIdentity::BadLeadingChar {
            kind,
            value: value.to_string(),
        });
    }
    if value == "." || value == ".." || value.contains("..") {
        return Err(InvalidIdentity::BadDotSegment {
            kind,
            value: value.to_string(),
        });
    }
    let bad_char = value.bytes().any(|b| {
        !(b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-' || b == b'.')
    });
    if bad_char {
        return Err(InvalidIdentity::BadChar {
            kind,
            value: value.to_string(),
        });
    }
    if value.split('.').any(str::is_empty) {
        return Err(InvalidIdentity::BadDotSegment {
            kind,
            value: value.to_string(),
        });
    }
    Ok(())
}

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
        feed_field(&mut hasher, extension.as_str().as_bytes());
        feed_field(&mut hasher, extension_version.as_bytes());
        feed_field(&mut hasher, local.as_str().as_bytes());
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
        use std::fmt::Write;
        let mut s = String::with_capacity(64);
        for byte in self.0 {
            write!(s, "{byte:02x}").expect("writing to String never fails"); // safety: std::fmt::Write for String is infallible
        }
        s
    }
}

impl fmt::Debug for HookId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Display only the first 4 bytes for log readability; full hex via
        // to_hex(). Avoids dumping 64-char strings into trace logs.
        write!(f, "HookId(")?;
        for byte in self.0.iter().take(4) {
            write!(f, "{byte:02x}")?;
        }
        write!(f, "…)")
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
///
/// **Two `ExtensionId` types coexist in the system, by design**:
///
/// - [`ironclaw_host_api::ExtensionId`] is the *authority-bearing* identifier:
///   validated at construction, compared and trusted across the host.
/// - `ironclaw_hooks::identity::ExtensionId` (this type) is a transparent
///   string newtype consumed by [`HookId::derive`] as input to the blake3
///   content-addressing hash.
///
/// The framework's [`crate::HookRegistrar`] already mirrors the host-api
/// type into this one when installing manifest entries. Authors building
/// hook IDs by hand (typically Trusted in-process hooks installed
/// outside the registrar) can use the [`From`] impl to convert:
///
/// ```ignore
/// let host_ext: ironclaw_host_api::ExtensionId = /* ... */;
/// let identity_ext: ironclaw_hooks::identity::ExtensionId = (&host_ext).into();
/// let id = HookId::derive(&identity_ext, "1.0.0", &local, HookVersion::ONE);
/// ```
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct ExtensionId(String);

impl ExtensionId {
    /// Construct a new `ExtensionId`, validating that the value meets the
    /// identity segment rules.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidIdentity> {
        let value = value.into();
        validate_identity_segment("extension", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Canonical newtype-template accessor for taking the inner string by
    /// value. Prefer this over [`Self::into_string`] in new code.
    pub fn into_inner(self) -> String {
        self.0
    }

    /// Legacy alias retained for callers that predate the canonical
    /// `into_inner` convention from `.claude/rules/types.md`. New code
    /// should use [`Self::into_inner`].
    pub fn into_string(self) -> String {
        self.into_inner()
    }
}

impl AsRef<str> for ExtensionId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<ExtensionId> for String {
    fn from(id: ExtensionId) -> Self {
        id.0
    }
}

impl fmt::Display for ExtensionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ExtensionId {
    type Error = InvalidIdentity;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for ExtensionId {
    type Error = InvalidIdentity;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<&ironclaw_host_api::ExtensionId> for ExtensionId {
    fn from(host: &ironclaw_host_api::ExtensionId) -> Self {
        // The host-api `ExtensionId` is already validated against the same
        // segment grammar that `validate_identity_segment` enforces here
        // (both mirror `ironclaw_host_api::ids::validate_name_segment`).
        // We still go through `new` to keep a single validation path. The
        // round-trip is asserted by `extension_id_from_host_api_round_trips`
        // and `extension_id_from_host_api_round_trips_grammar_corners` below;
        // if the two grammars ever diverge those tests will fail before this
        // call site can panic in production.
        let msg = "ironclaw_host_api::ExtensionId is pre-validated and shares the identity grammar";
        Self::new(host.as_str().to_string()).expect(msg) // safety: host-api ExtensionId shares identity grammar; round-trip covered by unit tests above.
    }
}

/// Extension-author-chosen identifier for the hook within their manifest.
/// Combined with `ExtensionId` and versions to form a globally-unique `HookId`.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct HookLocalId(String);

impl HookLocalId {
    /// Construct a new `HookLocalId`, validating that the value meets the
    /// identity segment rules.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidIdentity> {
        let value = value.into();
        validate_identity_segment("hook_local", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Canonical newtype-template accessor for taking the inner string by
    /// value. Prefer this over [`Self::into_string`] in new code.
    pub fn into_inner(self) -> String {
        self.0
    }

    /// Legacy alias retained for callers that predate the canonical
    /// `into_inner` convention from `.claude/rules/types.md`. New code
    /// should use [`Self::into_inner`].
    pub fn into_string(self) -> String {
        self.into_inner()
    }
}

impl AsRef<str> for HookLocalId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<HookLocalId> for String {
    fn from(id: HookLocalId) -> Self {
        id.0
    }
}

impl fmt::Display for HookLocalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for HookLocalId {
    type Error = InvalidIdentity;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for HookLocalId {
    type Error = InvalidIdentity;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

fn feed_field(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ext(s: &str) -> ExtensionId {
        ExtensionId::new(s).expect("test extension id is valid")
    }

    fn local(s: &str) -> HookLocalId {
        HookLocalId::new(s).expect("test hook local id is valid")
    }

    #[test]
    fn derive_is_deterministic() {
        let a = HookId::derive(
            &ext("polymarket-trader"),
            "0.4.2",
            &local("daily-order-cap"),
            HookVersion::ONE,
        );
        let b = HookId::derive(
            &ext("polymarket-trader"),
            "0.4.2",
            &local("daily-order-cap"),
            HookVersion::ONE,
        );
        assert_eq!(a, b);
    }

    #[test]
    fn version_bump_changes_id() {
        let a = HookId::derive(&ext("ext"), "1.0", &local("h"), HookVersion(1));
        let b = HookId::derive(&ext("ext"), "1.0", &local("h"), HookVersion(2));
        assert_ne!(a, b);
    }

    #[test]
    fn extension_version_bump_changes_id() {
        let a = HookId::derive(&ext("ext"), "1.0", &local("h"), HookVersion::ONE);
        let b = HookId::derive(&ext("ext"), "1.1", &local("h"), HookVersion::ONE);
        assert_ne!(a, b);
    }

    #[test]
    fn length_prefix_prevents_field_concatenation_collision() {
        // Without length-prefixing, ("ab", "c") and ("a", "bc") would collide.
        // Length-prefixing must keep them distinct.
        let a = HookId::derive(&ext("ab"), "1.0", &local("c"), HookVersion::ONE);
        let b = HookId::derive(&ext("a"), "1.0", &local("bc"), HookVersion::ONE);
        assert_ne!(a, b);
    }

    #[test]
    fn builtin_id_distinct_from_extension_id() {
        // Use a `.`-separated local id, which is permitted by the segment
        // grammar (mirroring host-api's `validate_name_segment`). The
        // original fixture used `"path::module"` for the installed local id,
        // but the post-#3912 segment grammar disallows `:` characters
        // anywhere in a `HookLocalId`, so we substitute the equivalent
        // legal value `"path.module"`. The assertion's intent — that the
        // builtin and installed digests are distinct for any pair of
        // syntactically legal ids — is unchanged; `HookId::for_builtin`
        // accepts a free-form canonical path (not a `HookLocalId`) so the
        // builtin side can still carry the original `"path::module"`
        // shape.
        let installed = HookId::derive(
            &ext("builtin"),
            "x",
            &local("path.module"),
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
        let derived = HookId::derive(&ext("ext"), "1.0", &local("h"), HookVersion::ONE);
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

    // -----------------------------------------------------------------
    // Validation regression tests — these guard the invariants that
    // ExtensionId and HookLocalId enforce at construction time.
    // -----------------------------------------------------------------

    #[test]
    fn extension_id_rejects_empty() {
        assert!(matches!(
            ExtensionId::new(""),
            Err(InvalidIdentity::Empty { .. })
        ));
    }

    #[test]
    fn extension_id_rejects_oversized() {
        let too_long = "a".repeat(MAX_IDENTITY_BYTES + 1);
        assert!(matches!(
            ExtensionId::new(too_long),
            Err(InvalidIdentity::TooLong { .. })
        ));
    }

    #[test]
    fn extension_id_rejects_invalid_chars() {
        // Uppercase, slashes, NUL, spaces, colons — all rejected.
        for bad in ["Github", "ext/sub", "ext\0nul", "ext name", "ext:sub"] {
            assert!(
                ExtensionId::new(bad).is_err(),
                "expected `{bad}` to be rejected"
            );
        }
    }

    #[test]
    fn extension_id_rejects_bad_leading_char() {
        for bad in ["-leading-dash", "_leading-underscore", ".leading-dot"] {
            assert!(
                matches!(
                    ExtensionId::new(bad),
                    Err(InvalidIdentity::BadLeadingChar { .. })
                ),
                "expected leading-char rejection for `{bad}`"
            );
        }
    }

    #[test]
    fn extension_id_rejects_dot_dot_and_empty_segments() {
        for bad in ["..", "a..b", "a.", ".a"] {
            assert!(
                ExtensionId::new(bad).is_err(),
                "expected `{bad}` to be rejected (dot/segment rules)"
            );
        }
    }

    #[test]
    fn extension_id_accepts_valid_value() {
        for ok in [
            "github",
            "github-mcp.v1",
            "0day",
            "a",
            "ext_with_underscore",
        ] {
            assert!(
                ExtensionId::new(ok).is_ok(),
                "expected `{ok}` to be accepted"
            );
        }
    }

    #[test]
    fn hook_local_id_rejects_empty() {
        assert!(matches!(
            HookLocalId::new(""),
            Err(InvalidIdentity::Empty { .. })
        ));
    }

    #[test]
    fn hook_local_id_rejects_oversized() {
        let too_long = "a".repeat(MAX_IDENTITY_BYTES + 1);
        assert!(matches!(
            HookLocalId::new(too_long),
            Err(InvalidIdentity::TooLong { .. })
        ));
    }

    #[test]
    fn hook_local_id_rejects_invalid_chars() {
        for bad in ["Daily", "h::path", "h/sub", "h\0nul", "h space"] {
            assert!(
                HookLocalId::new(bad).is_err(),
                "expected `{bad}` to be rejected"
            );
        }
    }

    #[test]
    fn hook_local_id_accepts_valid_value() {
        for ok in ["daily-order-cap", "h", "path.module", "v2_handler"] {
            assert!(
                HookLocalId::new(ok).is_ok(),
                "expected `{ok}` to be accepted"
            );
        }
    }

    #[test]
    fn extension_id_deserialize_fails_closed_on_invalid_input() {
        let err = serde_json::from_str::<ExtensionId>("\"Bad/Value\"")
            .expect_err("uppercase and slash must reject");
        assert!(
            err.to_string().to_lowercase().contains("extension"),
            "error should mention `extension`: {err}"
        );
    }

    #[test]
    fn extension_id_deserialize_accepts_valid_input() {
        let id: ExtensionId =
            serde_json::from_str("\"github-mcp.v1\"").expect("valid extension id must deserialize");
        assert_eq!(id.as_str(), "github-mcp.v1");
    }

    #[test]
    fn hook_local_id_deserialize_fails_closed_on_invalid_input() {
        let err = serde_json::from_str::<HookLocalId>("\"\"").expect_err("empty must reject");
        assert!(err.to_string().to_lowercase().contains("hook_local"));
    }

    #[test]
    fn hook_local_id_deserialize_accepts_valid_input() {
        let id: HookLocalId =
            serde_json::from_str("\"daily-cap\"").expect("valid hook local id must deserialize");
        assert_eq!(id.as_str(), "daily-cap");
    }

    #[test]
    fn extension_id_from_host_api_round_trips() {
        let host = ironclaw_host_api::ExtensionId::new("github-mcp.v1").expect("valid host id");
        let mirrored: ExtensionId = (&host).into();
        assert_eq!(mirrored.as_str(), "github-mcp.v1");
    }

    /// Walks the grammar corners that `validate_name_segment` and
    /// `validate_identity_segment` both accept, asserting the host-api ->
    /// identity round-trip is infallible at each boundary. This is the
    /// regression guard that backs the `expect()` in
    /// `From<&ironclaw_host_api::ExtensionId> for ExtensionId`: if the two
    /// validators ever drift apart, one of these constructions will fail
    /// the host-api `::new` call (because that path runs first) and surface
    /// the divergence here instead of panicking in production.
    #[test]
    fn extension_id_from_host_api_round_trips_grammar_corners() {
        let corners = [
            "a",                             // 1-byte minimum
            "0",                             // leading digit
            "abc",                           // plain lowercase
            "abc-def",                       // dash
            "abc_def",                       // underscore
            "abc.def",                       // single dot
            "github-mcp.v1",                 // dash + dot
            "a.b.c.d.e",                     // multiple dot segments
            "0123456789",                    // digits only
            &"a".repeat(MAX_IDENTITY_BYTES), // max length
        ];
        for raw in corners {
            let host = ironclaw_host_api::ExtensionId::new(raw)
                .unwrap_or_else(|e| panic!("host-api rejected corner {raw:?}: {e}"));
            let mirrored: ExtensionId = (&host).into();
            assert_eq!(mirrored.as_str(), raw);
        }
    }
}
