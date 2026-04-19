//! Typed identifiers for internal names.
//!
//! Two string-shaped values that must not be confused:
//!
//! - [`CredentialName`] — backend secret identity used for storage, injection,
//!   and gate resume (e.g. `telegram_bot_token`, `google_oauth_token`).
//! - [`ExtensionName`] — user-facing installed extension/channel identity used
//!   for setup routing, UI, and Python action dispatch (e.g. `telegram`,
//!   `gmail`).
//!
//! See `.claude/rules/types.md` for why these are newtypes and
//! `CLAUDE.md` → "Extension/Auth Invariants" for the routing rules.
//!
//! # Wire compatibility
//!
//! Both types use `#[serde(transparent)]` so the on-wire and on-disk
//! representation is a plain JSON string — unchanged from when the fields
//! were `String`. Validation runs only when constructing through the
//! validated entry points (`new` / `try_from` / `from_str`), not at
//! deserialize time. Legacy persisted rows therefore continue to
//! deserialize cleanly; an invalid value is only surfaced if a later
//! code path re-constructs the name through a validated entry point.
//! There is no re-validation API on an existing instance — by design,
//! the type represents "something that passed validation at some point
//! in its history" rather than "something guaranteed valid right now".

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Shared maximum length for both credential and extension names.
///
/// Matches the pre-newtype `is_valid_credential_name` bound; extension names
/// had no explicit length cap but fit comfortably within this limit in
/// practice.
pub const MAX_NAME_LEN: usize = 64;

/// Why a candidate string is not a valid identity name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdentityError {
    #[error("identity name must not be empty")]
    Empty,
    #[error("identity name '{0}' exceeds {MAX_NAME_LEN} characters")]
    TooLong(String),
    #[error("identity name '{0}': must not contain path separators or traversal characters")]
    PathTraversal(String),
    #[error("identity name '{0}': only lowercase letters, digits, and underscores are allowed")]
    InvalidChar(String),
    #[error("identity name '{0}': must start and end with a lowercase letter or digit")]
    EdgeUnderscore(String),
    #[error("identity name '{0}': consecutive underscores are not allowed")]
    ConsecutiveUnderscores(String),
}

/// Validate `raw` against the shared rule and return its canonical form.
///
/// The canonical form trims surrounding whitespace and replaces `-` with `_`
/// (extension names are invoked as Python attribute accesses, which forbid
/// hyphens). After that normalization the result must be:
///
/// - non-empty, at most [`MAX_NAME_LEN`] bytes
/// - ASCII lowercase letters, digits, and `_` only
/// - not start or end with `_`
/// - no consecutive `__`
/// - no path separators (`/`, `\`), parent-traversal (`..`), or NUL
///
/// Checks are ordered cheapest-first against the trimmed slice so that an
/// invalid input rejects without allocating a canonicalized `String`.
/// `replace('-', "_")` runs only after the structural checks pass; since
/// `-` and `_` are both one byte, it cannot change the already-checked
/// length, so the fast-path length check stays valid.
fn canonicalize(raw: &str) -> Result<String, IdentityError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(IdentityError::Empty);
    }
    if trimmed.len() > MAX_NAME_LEN {
        return Err(IdentityError::TooLong(trimmed.to_string()));
    }
    if trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
        || trimmed.contains('\0')
    {
        return Err(IdentityError::PathTraversal(trimmed.to_string()));
    }

    let canonical = trimmed.replace('-', "_");
    let bytes = canonical.as_bytes();
    if bytes.first() == Some(&b'_') || bytes.last() == Some(&b'_') {
        return Err(IdentityError::EdgeUnderscore(canonical));
    }

    let mut prev_underscore = false;
    for ch in canonical.chars() {
        let is_valid = ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_';
        if !is_valid {
            return Err(IdentityError::InvalidChar(canonical));
        }
        if ch == '_' {
            if prev_underscore {
                return Err(IdentityError::ConsecutiveUnderscores(canonical));
            }
            prev_underscore = true;
        } else {
            prev_underscore = false;
        }
    }

    Ok(canonical)
}

macro_rules! identity_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Debug,
            Clone,
            PartialEq,
            Eq,
            Hash,
            PartialOrd,
            Ord,
            Serialize,
            Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Construct from any string-like value, validating + canonicalizing.
            pub fn new(raw: impl AsRef<str>) -> Result<Self, IdentityError> {
                canonicalize(raw.as_ref()).map(Self)
            }

            /// Construct without validation.
            ///
            /// Use for values sourced from a typed upstream that the caller
            /// already trusts — a DB row, a skill-manifest registry entry,
            /// a `#[serde(transparent)]` deserialization whose wire contract
            /// predates the newtype. Prefer [`Self::new`] for anything
            /// touching user input, free-form text, or external-tool output.
            pub fn from_trusted(raw: String) -> Self {
                Self(raw)
            }

            /// Borrow the inner canonical string.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Consume and return the inner `String`.
            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        // `AsRef<str>` is intentionally implemented so callers can opt into
        // a `&str` view through a method call (`.as_ref()` / `.as_str()`),
        // which makes the boundary crossing visible in the source. We do
        // *not* implement `Deref<Target = str>`: auto-deref would let
        // `&credential_name` silently coerce to `&str`, which is exactly the
        // implicit-conversion behaviour these newtypes exist to prevent.
        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<&str> for $name {
            type Error = IdentityError;
            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdentityError;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl FromStr for $name {
            type Err = IdentityError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::new(s)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> String {
                value.0
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.0 == *other
            }
        }
    };
}

identity_newtype! {
    /// Backend secret identity — e.g. `telegram_bot_token`, `google_oauth_token`.
    ///
    /// Used as the lookup key in the secrets store, in gate resume payloads,
    /// and anywhere the system needs to refer to *which* credential slot is
    /// being filled. Must not be used as a UI routing key — that is
    /// [`ExtensionName`]'s job.
    CredentialName
}

identity_newtype! {
    /// User-facing extension/channel identity — e.g. `telegram`, `gmail`.
    ///
    /// Used to route onboarding UI, setup/configure modals, and Python action
    /// dispatch. Hyphens in input are folded to underscores at construction
    /// time because extensions are invoked as attribute accesses in the
    /// embedded Python interpreter. Must not be used as a secrets-store key —
    /// that is [`CredentialName`]'s job.
    ExtensionName
}

impl ExtensionName {
    /// The pre-v0.23 hyphenated variant of this name, if one exists.
    ///
    /// Returns `Some("google-calendar")` for `google_calendar`, `None` for
    /// names without underscores. Used when locating older release artifacts
    /// on disk.
    pub fn legacy_alias(&self) -> Option<String> {
        let alias = self.0.replace('_', "-");
        (alias != self.0).then_some(alias)
    }
}

/// Maximum length for an [`ExternalThreadId`].
///
/// Chosen to accommodate Slack's compound `thread_ts` identifiers, web-UI
/// generated UUID strings, Telegram chat IDs, and comparable channel-specific
/// thread tokens, while still bounding what we'll accept from an external
/// system.
pub const MAX_EXTERNAL_THREAD_ID_LEN: usize = 512;

/// Why a candidate string is not a valid external thread id.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExternalThreadIdError {
    #[error("external thread id must not be empty")]
    Empty,
    #[error("external thread id exceeds {MAX_EXTERNAL_THREAD_ID_LEN} characters")]
    TooLong,
    #[error("external thread id must not contain NUL bytes")]
    ContainsNul,
}

/// External (channel-supplied) thread identifier — e.g. a Telegram chat id,
/// a Slack `thread_ts`, a web-UI-generated UUID string.
///
/// **Not** the internal engine `ThreadId(Uuid)`. Channels supply whatever
/// shape their platform uses; [`crate::identity::ExternalThreadId`] is the
/// typed boundary representation that carries that raw string safely across
/// internal module boundaries. Conversion to an internal UUID happens inside
/// `SessionManager::resolve_thread` and equivalents.
///
/// See `.claude/rules/types.md` for why this is a newtype.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExternalThreadId(String);

impl ExternalThreadId {
    /// Construct from any string-like value, validating length and
    /// disallowing NUL bytes. Returns [`ExternalThreadIdError`] on failure.
    pub fn new(raw: impl AsRef<str>) -> Result<Self, ExternalThreadIdError> {
        let s = raw.as_ref();
        if s.is_empty() {
            return Err(ExternalThreadIdError::Empty);
        }
        if s.len() > MAX_EXTERNAL_THREAD_ID_LEN {
            return Err(ExternalThreadIdError::TooLong);
        }
        if s.contains('\0') {
            return Err(ExternalThreadIdError::ContainsNul);
        }
        Ok(Self(s.to_string()))
    }

    /// Construct without validation.
    ///
    /// Use for values sourced from a typed upstream that the caller already
    /// trusts — a DB row, a persisted pending-gate payload, or a
    /// `#[serde(transparent)]` deserialization whose wire contract predates
    /// the newtype. Prefer [`Self::new`] for anything touching external input.
    pub fn from_trusted(raw: String) -> Self {
        Self(raw)
    }

    /// Borrow the inner string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume and return the inner `String`.
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for ExternalThreadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// Intentionally no `Deref<Target = str>`, no `From<String>`, no
// `From<&str>`: the whole point of this newtype is to force callers to
// make the boundary crossing explicit via `new` (validating) or
// `from_trusted` (documented opt-out).
impl AsRef<str> for ExternalThreadId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for ExternalThreadId {
    type Error = ExternalThreadIdError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for ExternalThreadId {
    type Error = ExternalThreadIdError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl FromStr for ExternalThreadId {
    type Err = ExternalThreadIdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl From<ExternalThreadId> for String {
    fn from(value: ExternalThreadId) -> String {
        value.0
    }
}

impl PartialEq<str> for ExternalThreadId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for ExternalThreadId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// Maximum length for an [`McpServerName`].
///
/// MCP server names are used as tool-name prefixes in LLM providers (which
/// typically require `^[a-zA-Z0-9_-]+$`), as components of secret-store keys
/// (e.g. `mcp_<name>_access_token`), and as filesystem-adjacent identifiers.
/// 64 bytes matches the shared `MAX_NAME_LEN` used for other identity names.
pub const MAX_MCP_SERVER_NAME_LEN: usize = 64;

/// Why a candidate string is not a valid MCP server name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum McpServerNameError {
    #[error("MCP server name must not be empty")]
    Empty,
    #[error("MCP server name exceeds {MAX_MCP_SERVER_NAME_LEN} characters")]
    TooLong,
    #[error(
        "MCP server name '{0}' contains invalid characters \
         (only alphanumeric, dash, underscore are allowed)"
    )]
    InvalidChar(String),
}

/// MCP server identifier — e.g. `notion`, `github`, `my-server`.
///
/// The allowlist rules mirror the pre-newtype check that landed in #2400:
/// alphanumeric, dash, and underscore only. These rules are intentionally
/// more permissive than [`CredentialName`] / [`ExtensionName`] because MCP
/// server names were historically free-form — we reject shell metacharacters
/// and path separators but still accept uppercase letters and dashes. The
/// character set is a superset of what LLM providers accept for tool-name
/// prefixes (`^[a-zA-Z0-9_-]+$`).
///
/// Callers must go through [`Self::new`] (validating) or
/// [`Self::from_trusted`] (documented opt-out, e.g. for values already
/// validated at load time). Deliberately no `From<String>` / `From<&str>`.
///
/// `#[serde(transparent)]` preserves on-wire compatibility — legacy config
/// rows continue to deserialize cleanly, and invalid values are only
/// surfaced when re-validated through [`Self::new`]. See
/// `.claude/rules/types.md`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct McpServerName(String);

impl McpServerName {
    /// Construct from any string-like value, validating the allowlist.
    ///
    /// Rejects empty strings, strings longer than
    /// [`MAX_MCP_SERVER_NAME_LEN`], and strings containing any character
    /// outside the allowlist (alphanumeric, `-`, `_`). Path separators,
    /// shell metacharacters, NUL bytes, and whitespace all fall into the
    /// invalid-character bucket.
    pub fn new(raw: impl AsRef<str>) -> Result<Self, McpServerNameError> {
        let s = raw.as_ref();
        if s.is_empty() {
            return Err(McpServerNameError::Empty);
        }
        if s.len() > MAX_MCP_SERVER_NAME_LEN {
            return Err(McpServerNameError::TooLong);
        }
        if !s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(McpServerNameError::InvalidChar(s.to_string()));
        }
        Ok(Self(s.to_string()))
    }

    /// Construct without validation.
    ///
    /// Use for values sourced from a typed upstream that the caller already
    /// trusts — an already-validated config row, a canonicalised name after
    /// hyphen-to-underscore folding in the factory, or a
    /// `#[serde(transparent)]` deserialization whose wire contract predates
    /// the newtype. Prefer [`Self::new`] for anything touching external
    /// input.
    pub fn from_trusted(raw: String) -> Self {
        Self(raw)
    }

    /// Borrow the inner string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume and return the inner `String`.
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for McpServerName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// Intentionally no `Deref<Target = str>`, no `From<String>`, no
// `From<&str>`: the whole point of this newtype is to force callers to
// make the boundary crossing explicit via `new` (validating) or
// `from_trusted` (documented opt-out).
impl AsRef<str> for McpServerName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for McpServerName {
    type Error = McpServerNameError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for McpServerName {
    type Error = McpServerNameError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl FromStr for McpServerName {
    type Err = McpServerNameError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl From<McpServerName> for String {
    fn from(value: McpServerName) -> String {
        value.0
    }
}

impl PartialEq<str> for McpServerName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for McpServerName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_snake_case() {
        assert_eq!(
            ExtensionName::new("google_drive").unwrap().as_str(),
            "google_drive"
        );
        assert_eq!(
            CredentialName::new("telegram_bot_token").unwrap().as_str(),
            "telegram_bot_token"
        );
    }

    #[test]
    fn folds_hyphens_to_underscores() {
        assert_eq!(
            ExtensionName::new("web-search").unwrap().as_str(),
            "web_search"
        );
        assert_eq!(
            CredentialName::new("github-token").unwrap().as_str(),
            "github_token"
        );
    }

    #[test]
    fn trims_whitespace() {
        assert_eq!(ExtensionName::new("  gmail  ").unwrap().as_str(), "gmail");
    }

    #[test]
    fn rejects_empty_and_whitespace_only() {
        assert_eq!(ExtensionName::new(""), Err(IdentityError::Empty));
        assert_eq!(ExtensionName::new("   "), Err(IdentityError::Empty));
    }

    #[test]
    fn rejects_uppercase() {
        assert!(matches!(
            ExtensionName::new("WebSearch"),
            Err(IdentityError::InvalidChar(_))
        ));
        assert!(matches!(
            CredentialName::new("GitHub_Token"),
            Err(IdentityError::InvalidChar(_))
        ));
    }

    #[test]
    fn rejects_consecutive_underscores() {
        assert!(matches!(
            ExtensionName::new("bad__name"),
            Err(IdentityError::ConsecutiveUnderscores(_))
        ));
    }

    #[test]
    fn rejects_edge_underscores() {
        assert!(matches!(
            ExtensionName::new("_leading"),
            Err(IdentityError::EdgeUnderscore(_))
        ));
        assert!(matches!(
            ExtensionName::new("trailing_"),
            Err(IdentityError::EdgeUnderscore(_))
        ));
    }

    #[test]
    fn rejects_path_traversal() {
        assert!(matches!(
            ExtensionName::new("../bad"),
            Err(IdentityError::PathTraversal(_))
        ));
        assert!(matches!(
            ExtensionName::new("a/b"),
            Err(IdentityError::PathTraversal(_))
        ));
        assert!(matches!(
            ExtensionName::new("with\0nul"),
            Err(IdentityError::PathTraversal(_))
        ));
    }

    #[test]
    fn rejects_too_long() {
        let long = "a".repeat(MAX_NAME_LEN + 1);
        assert!(matches!(
            ExtensionName::new(&long),
            Err(IdentityError::TooLong(_))
        ));
    }

    #[test]
    fn rejects_invalid_chars() {
        assert!(matches!(
            CredentialName::new("foo bar"),
            Err(IdentityError::InvalidChar(_))
        ));
        assert!(matches!(
            CredentialName::new("foo.bar"),
            Err(IdentityError::InvalidChar(_))
        ));
    }

    #[test]
    fn serde_is_transparent() {
        let ext = ExtensionName::new("gmail").unwrap();
        let json = serde_json::to_string(&ext).unwrap();
        assert_eq!(json, "\"gmail\"");

        let round: ExtensionName = serde_json::from_str("\"gmail\"").unwrap();
        assert_eq!(round.as_str(), "gmail");
    }

    /// `#[serde(transparent)]` means we do not re-validate at deserialize
    /// time — legacy persisted rows must keep loading. Validation happens at
    /// construction sites, not on the wire.
    #[test]
    fn serde_does_not_revalidate() {
        let legacy: ExtensionName = serde_json::from_str("\"Bad__Name\"").unwrap();
        assert_eq!(legacy.as_str(), "Bad__Name");
    }

    #[test]
    fn credential_and_extension_are_distinct_types() {
        let cred = CredentialName::new("github_token").unwrap();
        let ext = ExtensionName::new("github").unwrap();

        // Compile-time check — passing one where the other is expected must
        // not compile. We assert the runtime shape and trust the type system
        // for the rest.
        assert_eq!(cred.as_str(), "github_token");
        assert_eq!(ext.as_str(), "github");
    }

    #[test]
    fn legacy_alias_roundtrip() {
        let ext = ExtensionName::new("google_calendar").unwrap();
        assert_eq!(ext.legacy_alias().as_deref(), Some("google-calendar"));

        let no_underscore = ExtensionName::new("gmail").unwrap();
        assert_eq!(no_underscore.legacy_alias(), None);
    }

    #[test]
    fn display_matches_inner() {
        let ext = ExtensionName::new("gmail").unwrap();
        assert_eq!(format!("{ext}"), "gmail");
    }

    #[test]
    fn partial_eq_with_str() {
        let ext = ExtensionName::new("gmail").unwrap();
        assert_eq!(ext, *"gmail");
        assert_eq!(ext, "gmail");
    }

    /// Guards the decision to *not* implement `Deref<Target = str>`:
    /// auto-deref would let `&ext_name` silently coerce to `&str`, which
    /// is the implicit-conversion pattern the newtypes exist to prevent.
    /// Callers must go through `.as_str()` / `.as_ref()` — both explicit.
    /// If a future edit adds `Deref`, this test will still compile but
    /// the doc contract is broken; the rule lives in
    /// `.claude/rules/types.md`.
    #[test]
    fn explicit_accessors_work() {
        let ext = ExtensionName::new("gmail").unwrap();
        let via_as_str: &str = ext.as_str();
        let via_as_ref: &str = ext.as_ref();
        assert_eq!(via_as_str, "gmail");
        assert_eq!(via_as_ref, "gmail");
    }

    // ---- ExternalThreadId tests ----

    #[test]
    fn external_thread_id_accepts_common_channel_shapes() {
        // Telegram-style numeric chat id
        assert_eq!(
            ExternalThreadId::new("123456789").unwrap().as_str(),
            "123456789"
        );
        // Web UI UUID
        assert_eq!(
            ExternalThreadId::new("550e8400-e29b-41d4-a716-446655440000")
                .unwrap()
                .as_str(),
            "550e8400-e29b-41d4-a716-446655440000"
        );
        // Slack thread_ts
        assert_eq!(
            ExternalThreadId::new("1234567890.123456").unwrap().as_str(),
            "1234567890.123456"
        );
        // Generic text with mixed punctuation — channels define shape
        assert!(ExternalThreadId::new("room:general").is_ok());
    }

    #[test]
    fn external_thread_id_rejects_empty() {
        assert_eq!(ExternalThreadId::new(""), Err(ExternalThreadIdError::Empty));
    }

    #[test]
    fn external_thread_id_rejects_too_long() {
        let long = "a".repeat(MAX_EXTERNAL_THREAD_ID_LEN + 1);
        assert_eq!(
            ExternalThreadId::new(&long),
            Err(ExternalThreadIdError::TooLong)
        );
    }

    #[test]
    fn external_thread_id_rejects_nul() {
        assert_eq!(
            ExternalThreadId::new("abc\0def"),
            Err(ExternalThreadIdError::ContainsNul)
        );
    }

    #[test]
    fn external_thread_id_serde_is_transparent() {
        let tid = ExternalThreadId::new("thread-xyz").unwrap();
        let json = serde_json::to_string(&tid).unwrap();
        assert_eq!(json, "\"thread-xyz\"");

        let round: ExternalThreadId = serde_json::from_str("\"thread-xyz\"").unwrap();
        assert_eq!(round.as_str(), "thread-xyz");
    }

    /// Like the other identity newtypes, `#[serde(transparent)]` means we
    /// do not re-validate at deserialize time — legacy persisted rows must
    /// keep loading. Validation happens at construction sites.
    #[test]
    fn external_thread_id_serde_does_not_revalidate() {
        // Even an empty string deserializes — we only reject via `new`.
        let legacy: ExternalThreadId = serde_json::from_str("\"\"").unwrap();
        assert_eq!(legacy.as_str(), "");
    }

    #[test]
    fn external_thread_id_from_trusted_preserves_raw() {
        let raw = "unvalidated::value".to_string();
        let tid = ExternalThreadId::from_trusted(raw.clone());
        assert_eq!(tid.as_str(), raw);
    }

    #[test]
    fn external_thread_id_distinct_from_extension_name() {
        let ext = ExtensionName::new("telegram").unwrap();
        let tid = ExternalThreadId::new("telegram").unwrap();
        // Compile-time distinction — both have the same inner shape but
        // are different types, so a function signature requiring one will
        // reject the other at the call site.
        assert_eq!(ext.as_str(), tid.as_str());
    }

    #[test]
    fn preserves_existing_credential_shape() {
        // Every credential name used in the codebase today (as of the
        // pre-newtype `parse_credential_name` tests) must still validate.
        for ok in [
            "github_token",
            "github_pat",
            "slack_token",
            "gmail_oauth",
            "linear_token",
            "telegram_bot_token",
            "google_oauth_token",
        ] {
            assert!(CredentialName::new(ok).is_ok(), "expected {ok} to validate",);
        }
    }

    // ---- McpServerName tests ----

    #[test]
    fn mcp_server_name_accepts_allowlist_characters() {
        // Alphanumeric, dashes, underscores, mixed case are all accepted —
        // this mirrors the pre-newtype `McpServerConfig::validate` coverage
        // (`test_server_name_valid_characters_accepted`).
        for ok in ["notion", "my-server", "my_server", "MCP-1", "server123"] {
            let name = McpServerName::new(ok).expect("should accept allowlist chars");
            assert_eq!(name.as_str(), ok);
        }
    }

    #[test]
    fn mcp_server_name_rejects_shell_metacharacters() {
        // Regression: the allowlist originated in #2400 as defence against
        // shell-metacharacter injection when the name is interpolated into
        // secret keys or tool-name prefixes.
        for bad in [
            "server; rm -rf /",
            "server$(whoami)",
            "server`id`",
            "server|cat /etc/passwd",
            "server&bg",
            "server>out",
            "server<in",
            "name with spaces",
        ] {
            assert!(
                matches!(
                    McpServerName::new(bad),
                    Err(McpServerNameError::InvalidChar(_))
                ),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn mcp_server_name_rejects_path_separators() {
        for bad in ["../etc/passwd", "server/name", "server\\name"] {
            assert!(
                matches!(
                    McpServerName::new(bad),
                    Err(McpServerNameError::InvalidChar(_))
                ),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn mcp_server_name_rejects_dots() {
        // Dots are rejected because server names are used as tool-name
        // prefixes and LLM providers require `^[a-zA-Z0-9_-]+$`.
        assert!(matches!(
            McpServerName::new("my.server"),
            Err(McpServerNameError::InvalidChar(_))
        ));
    }

    #[test]
    fn mcp_server_name_rejects_null_byte() {
        assert!(matches!(
            McpServerName::new("server\0name"),
            Err(McpServerNameError::InvalidChar(_))
        ));
    }

    #[test]
    fn mcp_server_name_rejects_empty() {
        assert_eq!(McpServerName::new(""), Err(McpServerNameError::Empty));
    }

    #[test]
    fn mcp_server_name_rejects_too_long() {
        let long = "a".repeat(MAX_MCP_SERVER_NAME_LEN + 1);
        assert_eq!(McpServerName::new(&long), Err(McpServerNameError::TooLong));
    }

    #[test]
    fn mcp_server_name_serde_is_transparent() {
        let name = McpServerName::new("notion").unwrap();
        let json = serde_json::to_string(&name).unwrap();
        assert_eq!(json, "\"notion\"");

        let round: McpServerName = serde_json::from_str("\"notion\"").unwrap();
        assert_eq!(round.as_str(), "notion");
    }

    /// Like the other identity newtypes, `#[serde(transparent)]` means we
    /// do not re-validate at deserialize time — legacy persisted
    /// `mcp-servers.json` rows must keep loading. Validation happens at
    /// construction sites (e.g. `McpServerConfig::validate`), not on the
    /// wire.
    #[test]
    fn mcp_server_name_serde_does_not_revalidate() {
        let legacy: McpServerName = serde_json::from_str("\"bad;server\"").unwrap();
        assert_eq!(legacy.as_str(), "bad;server");
    }

    #[test]
    fn mcp_server_name_from_trusted_preserves_raw() {
        // `from_trusted` is the documented escape hatch for canonicalised
        // post-validation values (e.g. after the factory folds hyphens).
        let name = McpServerName::from_trusted("my_server".to_string());
        assert_eq!(name.as_str(), "my_server");
    }
}
