//! Secret types for credential management.
//!
//! WASM tools NEVER see plaintext secrets. This module provides types
//! for secure storage and reference without exposing actual values.

use std::fmt;

use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A stored secret with encrypted value.
///
/// The plaintext is never stored; only the encrypted form exists in the database.
#[derive(Clone)]
pub struct Secret {
    pub id: Uuid,
    pub user_id: String,
    pub name: String,
    /// AES-256-GCM encrypted value (nonce || ciphertext || tag).
    pub encrypted_value: Vec<u8>,
    /// Per-secret salt for key derivation.
    pub key_salt: Vec<u8>,
    /// Optional provider hint (e.g., "openai", "stripe").
    pub provider: Option<String>,
    /// When this secret expires (None = never).
    pub expires_at: Option<DateTime<Utc>>,
    /// Last time this secret was used for injection.
    pub last_used_at: Option<DateTime<Utc>>,
    /// Total number of times this secret has been used.
    pub usage_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Secret")
            .field("id", &self.id)
            .field("user_id", &self.user_id)
            .field("name", &self.name)
            .field("encrypted_value", &"[REDACTED]")
            .field("key_salt", &"[REDACTED]")
            .field("provider", &self.provider)
            .field("expires_at", &self.expires_at)
            .field("last_used_at", &self.last_used_at)
            .field("usage_count", &self.usage_count)
            .finish()
    }
}

/// A reference to a secret by name, without exposing the value.
///
/// WASM tools receive these references and can check if secrets exist,
/// but they cannot read the actual values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretRef {
    pub name: String,
    pub provider: Option<String>,
}

impl SecretRef {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            provider: None,
        }
    }

    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }
}

/// A decrypted secret value, held in secure memory.
///
/// This type:
/// - Zeros memory on drop
/// - Never appears in Debug output
/// - Only exists briefly during credential injection
pub struct DecryptedSecret {
    value: SecretString,
}

impl DecryptedSecret {
    /// Create a new decrypted secret from raw bytes.
    ///
    /// The bytes are converted to a UTF-8 string. For binary secrets,
    /// consider base64 encoding before storage.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, SecretError> {
        // Convert to string, then wrap in SecretString
        let s = String::from_utf8(bytes).map_err(|_| SecretError::InvalidUtf8)?;
        Ok(Self {
            value: SecretString::from(s),
        })
    }

    /// Expose the secret value for injection.
    ///
    /// This is the ONLY way to access the plaintext. Use sparingly
    /// and ensure the exposed value isn't logged or persisted.
    pub fn expose(&self) -> &str {
        self.value.expose_secret()
    }

    /// Get the length of the secret without exposing it.
    pub fn len(&self) -> usize {
        self.value.expose_secret().len()
    }

    /// Check if the secret is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl fmt::Debug for DecryptedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DecryptedSecret([REDACTED, {} bytes])", self.len())
    }
}

impl Clone for DecryptedSecret {
    fn clone(&self) -> Self {
        Self {
            value: SecretString::from(self.value.expose_secret().to_string()),
        }
    }
}

/// Errors that can occur during secret operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum SecretError {
    #[error("Secret not found: {0}")]
    NotFound(String),

    #[error("Secret has expired")]
    Expired,

    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),

    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),

    #[error("Invalid master key")]
    InvalidMasterKey,

    #[error("Secret value is not valid UTF-8")]
    InvalidUtf8,

    #[error("Database error: {0}")]
    Database(String),

    #[error("Secret access denied for tool")]
    AccessDenied,

    #[error("Keychain error: {0}")]
    KeychainError(String),
}

/// Parameters for creating a new secret.
#[derive(Debug)]
pub struct CreateSecretParams {
    pub name: String,
    pub value: SecretString,
    pub provider: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl CreateSecretParams {
    /// Create new secret params. The name is normalized to lowercase for
    /// case-insensitive matching (capabilities.json uses lowercase names
    /// like `slack_bot_token`, but UIs may store `SLACK_BOT_TOKEN`).
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into().to_lowercase(),
            value: SecretString::from(value.into()),
            provider: None,
            expires_at: None,
        }
    }

    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    pub fn with_expiry(mut self, expires_at: DateTime<Utc>) -> Self {
        self.expires_at = Some(expires_at);
        self
    }
}

/// Where a credential should be injected in an HTTP request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CredentialLocation {
    /// Inject as Authorization header (e.g., "Bearer {secret}")
    #[default]
    AuthorizationBearer,
    /// Inject as Authorization header with Basic auth
    AuthorizationBasic { username: String },
    /// Inject as a custom header
    Header {
        name: String,
        prefix: Option<String>,
    },
    /// Inject as a query parameter
    QueryParam { name: String },
    /// Inject by replacing a placeholder in URL or body templates
    UrlPath { placeholder: String },
}

/// Mapping from a secret name to where it should be injected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialMapping {
    /// Name of the secret to use.
    pub secret_name: String,
    /// Where to inject the credential.
    pub location: CredentialLocation,
    /// Host patterns this credential applies to (glob syntax).
    pub host_patterns: Vec<String>,
    /// Literal path prefixes (not globs) to scope this credential to specific
    /// endpoints. When empty, matches all paths on the host. When set, the
    /// request path must match a prefix at a segment boundary (`/` or `?`).
    #[serde(default)]
    pub path_patterns: Vec<String>,
    /// When `true`, the tool may run without this credential — the host
    /// is allowed to skip the mapping if the secret cannot be resolved.
    /// **Defaults to `false` (required)** so a tool that simply declares
    /// a credential without explicitly opting into "optional" cannot be
    /// silently downgraded to an unauthenticated request.
    #[serde(default)]
    pub optional: bool,
}

impl CredentialMapping {
    /// Returns true if this mapping matches the given host and path.
    ///
    /// Path matching requires a segment boundary: the path must equal the
    /// prefix exactly, or the character after the prefix must be `/` or `?`.
    /// This prevents `/account/info-steal` from matching `/account/info`.
    pub fn matches(&self, host: &str, path: &str) -> bool {
        if !self.matches_host(host) {
            return false;
        }
        if self.path_patterns.is_empty() {
            return true;
        }
        self.path_patterns
            .iter()
            .any(|prefix| path_matches_prefix(path, prefix))
    }

    /// Check host patterns only (ignoring path_patterns).
    /// Used by `has_credentials_for_host` where we need to know if ANY
    /// credential exists for a host, regardless of path scoping.
    pub fn matches_host(&self, host: &str) -> bool {
        self.host_patterns
            .iter()
            .any(|pattern| host_matches_pattern(host, pattern))
    }

    pub fn bearer(secret_name: impl Into<String>, host_pattern: impl Into<String>) -> Self {
        Self {
            secret_name: secret_name.into(),
            location: CredentialLocation::AuthorizationBearer,
            host_patterns: vec![host_pattern.into()],
            path_patterns: Vec::new(),
            optional: false,
        }
    }

    pub fn header(
        secret_name: impl Into<String>,
        header_name: impl Into<String>,
        host_pattern: impl Into<String>,
    ) -> Self {
        Self {
            secret_name: secret_name.into(),
            location: CredentialLocation::Header {
                name: header_name.into(),
                prefix: None,
            },
            host_patterns: vec![host_pattern.into()],
            path_patterns: Vec::new(),
            optional: false,
        }
    }
}

/// Match `path` against a literal prefix with segment-boundary enforcement.
/// After the prefix, the path must end or continue with `/` or `?`.
///
/// A segment that decodes to `.` or `..` is rejected (traversal). This
/// covers both literal `..` and percent-encoded variants like `%2e%2e`,
/// `%2e`, `%2E.`, etc. — every form that a server might normalize to a
/// dot-segment before routing. Legitimate segments with embedded encoded
/// dots (e.g. `foo%2ebar` → `foo.bar`, `v1%2e2` → `v1.2`) are allowed
/// because the decoded segment is neither `.` nor `..`.
pub(crate) fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    if path_has_dot_segment(path) {
        return false;
    }
    let path = path.strip_suffix('/').unwrap_or(path);
    let prefix = prefix.strip_suffix('/').unwrap_or(prefix);
    if path == prefix {
        return true;
    }
    if path.len() > prefix.len() && path.starts_with(prefix) {
        let next_char = path.as_bytes()[prefix.len()];
        return next_char == b'/' || next_char == b'?';
    }
    false
}

/// Compute the specificity of `mapping` relative to `req_path` for precedence
/// ordering when multiple credential mappings match the same request. Longer
/// matching path prefix = more specific. Returns 0 for mappings with no
/// `path_patterns` (global scope).
///
/// Callers sort ascending by specificity so the most-specific mapping is
/// applied LAST — that makes it overwrite any conflicting headers from
/// less-specific mappings under a last-write-wins merge.
pub(crate) fn match_specificity(path_patterns: &[String], req_path: &str) -> usize {
    path_patterns
        .iter()
        .filter(|p| path_matches_prefix(req_path, p))
        .map(|p| p.len())
        .max()
        .unwrap_or(0)
}

/// Extract the path component of a URL for credential path-pattern matching.
///
/// Used by both WASM wrappers (`tools/wasm/wrapper.rs` and
/// `channels/wasm/wrapper.rs`) to feed `path_matches_prefix`. On parse
/// failure, returns an empty string and logs at debug level — an empty
/// string cannot match any non-empty prefix, so path-scoped credentials
/// fail closed (safe) rather than silently injecting on malformed URLs.
pub(crate) fn extract_url_path_for_matching(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(parsed) => parsed.path().to_string(),
        Err(err) => {
            tracing::debug!(
                url = %url,
                error = %err,
                "URL parse failed during path-scoped credential check; skipping injection"
            );
            String::new()
        }
    }
}

/// Returns true if any segment of `path` decodes (per RFC 3986 percent-
/// decoding of octets) to the traversal segments `.` or `..`, or embeds
/// a decoded `/` that a downstream server might use to re-split the path.
///
/// Legitimate segments with embedded encoded dots (`foo%2ebar`, `v1%2e2`)
/// are allowed because they decode to a normal literal. Encoded slashes
/// (`%2f`), however, are rejected whenever they appear in a segment that
/// we haven't already deemed a dot-segment — some servers (Tomcat with
/// `allowEncodedSlash=true`, older IIS, a handful of reverse proxies)
/// will normalize `%2f` back to `/` and then re-apply path routing,
/// effectively letting `/api/v1/%2e%2e%2fadmin` escape a `/api/v1` scope.
fn path_has_dot_segment(path: &str) -> bool {
    path.split('/').any(is_traversal_segment)
}

fn is_traversal_segment(segment: &str) -> bool {
    let decoded = percent_decode_bytes(segment.as_bytes());
    if matches!(decoded.as_slice(), b"." | b"..") {
        return true;
    }
    // Embedded decoded `/` would be interpreted as a path separator by any
    // server that re-splits after decoding. Treat the whole segment as
    // traversal: if we cannot be sure the final routed path stays inside
    // the declared scope, refuse to match.
    if decoded.contains(&b'/') {
        return true;
    }
    false
}

fn percent_decode_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Check if a hostname matches a pattern (supports `*.` wildcard and port stripping).
/// Comparison is case-insensitive per RFC 4343.
pub(crate) fn host_matches_pattern(host: &str, pattern: &str) -> bool {
    let host = &host.to_ascii_lowercase();
    let pattern = &pattern.to_ascii_lowercase();
    if pattern == host {
        return true;
    }
    if pattern.contains(':')
        && pattern
            .split(':')
            .next()
            .is_some_and(|pattern_host| pattern_host == host)
    {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix("*.")
        && host.ends_with(suffix)
        && host.len() > suffix.len()
    {
        let prefix = &host[..host.len() - suffix.len()];
        if prefix.ends_with('.') || prefix.is_empty() {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use crate::secrets::types::{CreateSecretParams, DecryptedSecret, SecretRef};

    #[test]
    fn test_secret_ref_creation() {
        let r = SecretRef::new("my_api_key").with_provider("openai");
        assert_eq!(r.name, "my_api_key");
        assert_eq!(r.provider, Some("openai".to_string()));
    }

    #[test]
    fn test_decrypted_secret_redaction() {
        let secret = DecryptedSecret::from_bytes(b"super_secret_value".to_vec()).unwrap();
        let debug_str = format!("{:?}", secret);
        assert!(!debug_str.contains("super_secret_value"));
        assert!(debug_str.contains("REDACTED"));
    }

    #[test]
    fn test_decrypted_secret_expose() {
        let secret = DecryptedSecret::from_bytes(b"test_value".to_vec()).unwrap();
        assert_eq!(secret.expose(), "test_value");
        assert_eq!(secret.len(), 10);
    }

    #[test]
    fn test_create_params() {
        let params = CreateSecretParams::new("key", "value").with_provider("stripe");
        assert_eq!(params.name, "key");
        assert_eq!(params.provider, Some("stripe".to_string()));
    }

    #[test]
    fn test_create_params_name_lowercased() {
        let params = CreateSecretParams::new("SLACK_BOT_TOKEN", "val");
        assert_eq!(params.name, "slack_bot_token");
    }

    #[test]
    fn test_create_params_with_expiry() {
        use chrono::Utc;
        let expiry = Utc::now();
        let params = CreateSecretParams::new("key", "val").with_expiry(expiry);
        assert_eq!(params.expires_at, Some(expiry));
    }

    #[test]
    fn test_secret_ref_without_provider() {
        let r = SecretRef::new("token");
        assert_eq!(r.name, "token");
        assert!(r.provider.is_none());
    }

    #[test]
    fn test_secret_ref_serde_roundtrip() {
        let original = SecretRef::new("api_key").with_provider("openai");
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: SecretRef = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, original.name);
        assert_eq!(deserialized.provider, original.provider);
    }

    #[test]
    fn test_secret_ref_serde_without_provider() {
        let original = SecretRef::new("bare_token");
        let json = serde_json::to_string(&original).unwrap();
        assert!(json.contains("\"provider\":null"));
        let deserialized: SecretRef = serde_json::from_str(&json).unwrap();
        assert!(deserialized.provider.is_none());
    }

    #[test]
    fn test_credential_location_serde_roundtrip_bearer() {
        use crate::secrets::types::CredentialLocation;
        let loc = CredentialLocation::AuthorizationBearer;
        let json = serde_json::to_string(&loc).unwrap();
        let back: CredentialLocation = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, CredentialLocation::AuthorizationBearer));
    }

    #[test]
    fn test_credential_location_serde_roundtrip_basic() {
        use crate::secrets::types::CredentialLocation;
        let loc = CredentialLocation::AuthorizationBasic {
            username: "admin".to_string(),
        };
        let json = serde_json::to_string(&loc).unwrap();
        let back: CredentialLocation = serde_json::from_str(&json).unwrap();
        match back {
            CredentialLocation::AuthorizationBasic { username } => {
                assert_eq!(username, "admin");
            }
            _ => panic!("expected AuthorizationBasic"),
        }
    }

    #[test]
    fn test_credential_location_serde_roundtrip_header() {
        use crate::secrets::types::CredentialLocation;
        let loc = CredentialLocation::Header {
            name: "X-Api-Key".to_string(),
            prefix: Some("Token".to_string()),
        };
        let json = serde_json::to_string(&loc).unwrap();
        let back: CredentialLocation = serde_json::from_str(&json).unwrap();
        match back {
            CredentialLocation::Header { name, prefix } => {
                assert_eq!(name, "X-Api-Key");
                assert_eq!(prefix, Some("Token".to_string()));
            }
            _ => panic!("expected Header"),
        }
    }

    #[test]
    fn test_credential_location_serde_roundtrip_query_param() {
        use crate::secrets::types::CredentialLocation;
        let loc = CredentialLocation::QueryParam {
            name: "access_token".to_string(),
        };
        let json = serde_json::to_string(&loc).unwrap();
        let back: CredentialLocation = serde_json::from_str(&json).unwrap();
        match back {
            CredentialLocation::QueryParam { name } => assert_eq!(name, "access_token"),
            _ => panic!("expected QueryParam"),
        }
    }

    #[test]
    fn test_credential_location_serde_roundtrip_url_path() {
        use crate::secrets::types::CredentialLocation;
        let loc = CredentialLocation::UrlPath {
            placeholder: "{api_key}".to_string(),
        };
        let json = serde_json::to_string(&loc).unwrap();
        let back: CredentialLocation = serde_json::from_str(&json).unwrap();
        match back {
            CredentialLocation::UrlPath { placeholder } => assert_eq!(placeholder, "{api_key}"),
            _ => panic!("expected UrlPath"),
        }
    }

    #[test]
    fn test_credential_location_default_is_bearer() {
        use crate::secrets::types::CredentialLocation;
        let loc = CredentialLocation::default();
        assert!(matches!(loc, CredentialLocation::AuthorizationBearer));
    }

    #[test]
    fn test_credential_mapping_bearer_constructor() {
        use crate::secrets::types::CredentialMapping;
        let m = CredentialMapping::bearer("my_token", "*.example.com");
        assert_eq!(m.secret_name, "my_token");
        assert!(matches!(
            m.location,
            crate::secrets::types::CredentialLocation::AuthorizationBearer
        ));
        assert_eq!(m.host_patterns, vec!["*.example.com".to_string()]);
    }

    #[test]
    fn test_credential_mapping_header_constructor() {
        use crate::secrets::types::CredentialMapping;
        let m = CredentialMapping::header("key", "X-Custom", "api.host.com");
        assert_eq!(m.secret_name, "key");
        match &m.location {
            crate::secrets::types::CredentialLocation::Header { name, prefix } => {
                assert_eq!(name, "X-Custom");
                assert!(prefix.is_none());
            }
            _ => panic!("expected Header"),
        }
        assert_eq!(m.host_patterns, vec!["api.host.com".to_string()]);
    }

    #[test]
    fn test_credential_mapping_serde_roundtrip() {
        use crate::secrets::types::CredentialMapping;
        let original = CredentialMapping::bearer("tok", "*.api.com");
        let json = serde_json::to_string(&original).unwrap();
        let back: CredentialMapping = serde_json::from_str(&json).unwrap();
        assert_eq!(back.secret_name, "tok");
        assert_eq!(back.host_patterns, vec!["*.api.com".to_string()]);
    }

    #[test]
    fn test_decrypted_secret_invalid_utf8() {
        let result = DecryptedSecret::from_bytes(vec![0xFF, 0xFE, 0x00]);
        assert!(result.is_err());
    }

    #[test]
    fn test_decrypted_secret_empty() {
        let secret = DecryptedSecret::from_bytes(Vec::new()).unwrap();
        assert!(secret.is_empty());
        assert_eq!(secret.len(), 0);
        assert_eq!(secret.expose(), "");
    }

    #[test]
    fn test_decrypted_secret_clone() {
        let original = DecryptedSecret::from_bytes(b"cloneable".to_vec()).unwrap();
        let cloned = original.clone();
        assert_eq!(cloned.expose(), "cloneable");
        assert_eq!(cloned.len(), original.len());
    }

    #[test]
    fn test_secret_debug_redacts_fields() {
        use chrono::Utc;
        use uuid::Uuid;
        let secret = crate::secrets::types::Secret {
            id: Uuid::nil(),
            user_id: "user1".to_string(),
            name: "test_key".to_string(),
            encrypted_value: vec![1, 2, 3],
            key_salt: vec![4, 5, 6],
            provider: Some("aws".to_string()),
            expires_at: None,
            last_used_at: None,
            usage_count: 5,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let debug = format!("{:?}", secret);
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains("[1, 2, 3]"));
        assert!(!debug.contains("[4, 5, 6]"));
        assert!(debug.contains("test_key"));
    }

    #[test]
    fn test_secret_error_display() {
        use crate::secrets::types::SecretError;
        assert_eq!(
            SecretError::NotFound("foo".into()).to_string(),
            "Secret not found: foo"
        );
        assert_eq!(SecretError::Expired.to_string(), "Secret has expired");
        assert_eq!(
            SecretError::InvalidMasterKey.to_string(),
            "Invalid master key"
        );
        assert_eq!(
            SecretError::InvalidUtf8.to_string(),
            "Secret value is not valid UTF-8"
        );
        assert_eq!(
            SecretError::AccessDenied.to_string(),
            "Secret access denied for tool"
        );
    }

    #[test]
    fn path_matches_prefix_segment_boundary() {
        use super::path_matches_prefix;
        assert!(path_matches_prefix("/api/v1", "/api/v1"));
        assert!(path_matches_prefix("/api/v1/", "/api/v1"));
        assert!(path_matches_prefix("/api/v1", "/api/v1/"));
        assert!(path_matches_prefix("/api/v1/users", "/api/v1"));
        assert!(path_matches_prefix("/api/v1?page=1", "/api/v1"));
        assert!(!path_matches_prefix("/api/v1-malicious", "/api/v1"));
        assert!(!path_matches_prefix("/account/info-steal", "/account/info"));
        assert!(!path_matches_prefix("/other", "/api/v1"));
        assert!(!path_matches_prefix("/public/../send-wire", "/public"));
        assert!(!path_matches_prefix("/a/../b", "/a"));
        assert!(path_matches_prefix("/api/..config", "/api"));
        assert!(path_matches_prefix("/api/version2..beta", "/api"));
    }

    #[test]
    fn path_matches_prefix_rejects_percent_encoded_dot_segments() {
        use super::path_matches_prefix;
        // Lowercase %2e%2e — classic IIS/Tomcat path-traversal smuggling
        assert!(!path_matches_prefix("/api/v1/%2e%2e/admin", "/api/v1"));
        // Mixed case — some decoders are case-insensitive
        assert!(!path_matches_prefix("/api/v1/%2E%2E/admin", "/api/v1"));
        assert!(!path_matches_prefix("/api/v1/%2e%2E/admin", "/api/v1"));
        // Single %2e as a single-dot segment variant
        assert!(!path_matches_prefix("/api/v1/%2e/admin", "/api/v1"));
        // Mixed literal + encoded: `.%2e` decodes to `..`
        assert!(!path_matches_prefix("/api/v1/.%2e/admin", "/api/v1"));
        assert!(!path_matches_prefix("/api/v1/%2e./admin", "/api/v1"));
    }

    #[test]
    fn path_matches_prefix_rejects_percent_encoded_slash_smuggling() {
        // Regression for Firat round-5 (#3126256056): `%2f` (encoded slash)
        // inside a segment can be decoded back to `/` by some servers,
        // turning e.g. `/api/v1/%2e%2e%2fadmin` into `/api/v1/../admin` at
        // routing time — escaping the `/api/v1` scope. Since we can't know
        // the downstream decoder's policy, refuse to match whenever a
        // decoded segment contains a `/`.
        use super::path_matches_prefix;
        // The canonical attack: %2e%2e%2fadmin → ../admin
        assert!(!path_matches_prefix("/api/v1/%2e%2e%2fadmin", "/api/v1"));
        assert!(!path_matches_prefix("/api/v1/%2E%2E%2Fadmin", "/api/v1"));
        // Embedded-slash variant inside a single segment — still rejected
        // because the decoded form would split.
        assert!(!path_matches_prefix("/api/v1/foo%2fbar", "/api/v1"));
        // Uppercase form.
        assert!(!path_matches_prefix("/api/v1/foo%2Fbar", "/api/v1"));
    }

    #[test]
    fn path_matches_prefix_allows_legit_embedded_encoded_dot() {
        use super::path_matches_prefix;
        // Regression for over-rejection (Firat round-4): encoded dots inside
        // an otherwise legitimate segment decode to a normal literal dot
        // character, not a dot-segment. These paths must be allowed.
        assert!(path_matches_prefix("/files/foo%2ebar", "/files"));
        assert!(path_matches_prefix("/releases/v1%2e2", "/releases"));
        assert!(path_matches_prefix("/api/v1/some.file", "/api/v1"));
        // Prefix equal to segment boundary still works.
        assert!(path_matches_prefix("/files/foo%2ebar.txt", "/files"));
    }

    #[test]
    fn match_specificity_ranks_longer_prefixes_higher() {
        use super::{CredentialMapping, match_specificity};
        use crate::secrets::CredentialLocation;
        let mapping = CredentialMapping {
            secret_name: "tok".into(),
            location: CredentialLocation::AuthorizationBearer,
            host_patterns: vec!["api.example.com".into()],
            path_patterns: vec!["/api".into(), "/api/v1/write".into()],
            optional: false,
        };
        // When multiple patterns match, specificity = longest matching prefix.
        assert_eq!(
            match_specificity(&mapping.path_patterns, "/api/v1/write/x"),
            "/api/v1/write".len()
        );
        // Only the shorter pattern matches.
        assert_eq!(
            match_specificity(&mapping.path_patterns, "/api/other"),
            "/api".len()
        );
        // No match at all.
        assert_eq!(match_specificity(&mapping.path_patterns, "/zzz"), 0);
        // Empty patterns (global) = 0.
        assert_eq!(match_specificity(&[], "/anything"), 0);
    }

    #[test]
    fn host_matches_pattern_case_insensitive() {
        use super::host_matches_pattern;
        assert!(host_matches_pattern("API.Example.COM", "api.example.com"));
        assert!(host_matches_pattern("api.example.com", "API.EXAMPLE.COM"));
        assert!(host_matches_pattern("sub.example.com", "*.example.com"));
        assert!(host_matches_pattern("SUB.Example.COM", "*.example.com"));
        assert!(!host_matches_pattern("example.com", "*.example.com"));
    }

    #[test]
    fn credential_mapping_matches_with_path() {
        use super::CredentialMapping;
        use crate::secrets::CredentialLocation;
        let m = CredentialMapping {
            secret_name: "token".into(),
            location: CredentialLocation::AuthorizationBearer,
            host_patterns: vec!["api.example.com".into()],
            path_patterns: vec!["/account/info".into(), "/exchange-rate".into()],
            optional: false,
        };
        assert!(m.matches("api.example.com", "/account/info"));
        assert!(m.matches("api.example.com", "/account/info/detail"));
        assert!(m.matches("api.example.com", "/exchange-rate"));
        assert!(m.matches("api.example.com", "/exchange-rate?from=USD"));
        assert!(!m.matches("api.example.com", "/send-wire"));
        assert!(!m.matches("api.example.com", "/account/info-steal"));
        assert!(!m.matches("other.com", "/account/info"));
        assert!(m.matches_host("api.example.com"));
    }

    #[test]
    fn credential_mapping_empty_paths_matches_all() {
        use super::CredentialMapping;
        use crate::secrets::CredentialLocation;
        let m = CredentialMapping {
            secret_name: "token".into(),
            location: CredentialLocation::AuthorizationBearer,
            host_patterns: vec!["api.example.com".into()],
            path_patterns: Vec::new(),
            optional: false,
        };
        assert!(m.matches("api.example.com", "/anything"));
        assert!(m.matches("api.example.com", "/"));
    }
}
