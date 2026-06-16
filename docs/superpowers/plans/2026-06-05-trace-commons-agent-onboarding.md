# Trace Commons Agent Onboarding (IronClaw side) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A user pastes a Trace Commons invite link into IronClaw chat; the agent gathers two consents and registers the client via `POST /v1/onboard`, after which trace uploads authenticate with a self-signed device-key JWT — culminating in one PR whose tool contract the designer's onboarding-flow UI can build on.

**Architecture:** A new `onboarding` module in `crates/ironclaw_reborn_traces` owns invite-URL parsing, per-(scope, tenant) Ed25519 device keypairs (staged under a pending path before the network call, promoted after), the onboard HTTP exchange with invite-origin trust anchoring, and the policy write. `StandingTraceContributionPolicy` gains `auth_mode`/`device_key_id`; the existing upload-claim refresh gains a `DeviceKey` branch that self-signs 60s workload JWTs. Two first-party capabilities (`trace_commons.onboard`, `trace_commons.status`) expose the flow to the reborn engine; the consent conversation lives in a prompt file.

**Tech Stack:** Rust; `ed25519-dalek` 2.x (workspace dep), `sha2`, `base64`, `reqwest`, `jsonwebtoken` (header validation only — signing is manual base64url + dalek), `axum` (dev-dep mock issuer), `serde`.

**Spec:** `docs/superpowers/specs/2026-06-05-trace-commons-agent-onboarding-design.md` — read it before starting. Server contract is published as TraceCommons/trace-commons-server#137; the wire types below must match it field-for-field.

**Branch:** work on `trace-commons-agent-onboarding` (already exists, contains the spec).

**Conventions that apply to every task** (from CLAUDE.md):
- No `.unwrap()`/`.expect()` in production code (tests are fine).
- Errors via `thiserror` enums, mapped with context.
- `debug!` not `info!` for internals.
- After each task's tests pass: `cargo fmt` before committing.
- Final gate before PR: `cargo clippy --all --benches --tests --examples --all-features` must be zero-warning.

## File Structure

```
crates/ironclaw_reborn_traces/
├── src/lib.rs                      # MODIFY: pub mod onboarding;
├── src/onboarding/
│   ├── mod.rs                      # CREATE: re-exports, OnboardError, onboard() orchestration
│   ├── invite.rs                   # CREATE: ParsedInvite + invite URL parsing
│   ├── device_key.rs               # CREATE: DeviceKeypair gen/store/load/promote + JWT signing
│   └── protocol.rs                 # CREATE: OnboardRequest/OnboardResponse wire types (mirrors server #137)
├── src/contribution.rs             # MODIFY: policy fields + DeviceKey branch in claim refresh
crates/ironclaw_host_runtime/
├── src/first_party_tools/
│   ├── mod.rs                      # MODIFY: register two new capabilities
│   └── trace_commons.rs            # CREATE: onboard + status capability handlers
crates/ironclaw_engine/prompts/builtin/
│   └── trace_commons_onboarding.md # CREATE: agent conversation guidance
```

`onboarding/` is a directory module so each unit (parsing, keys, wire types) stays independently testable and small; `contribution.rs` is already 8000+ lines — we add the minimum there (policy fields + one auth branch) and keep everything new out of it.

---

### Task 1: Onboarding wire types (`protocol.rs`)

The workspace has no `trace-commons-protocol` dependency; types are defined locally (as the existing upload-claim request/response are) and must match TraceCommons/trace-commons-server#137 exactly. A future migration to a shared crate is out of scope.

**Files:**
- Create: `crates/ironclaw_reborn_traces/src/onboarding/protocol.rs`
- Create: `crates/ironclaw_reborn_traces/src/onboarding/mod.rs` (skeleton: `pub mod protocol;`)
- Modify: `crates/ironclaw_reborn_traces/src/lib.rs` (add `pub mod onboarding;` after the existing `pub mod` lines)

- [ ] **Step 1: Write failing serde round-trip tests**

In `protocol.rs`, bottom of file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onboard_request_serializes_to_contract_shape() {
        let req = OnboardRequest {
            schema_version: ONBOARD_REQUEST_SCHEMA_VERSION,
            invite_code: "INV9K3RT5FBQ72JX".to_string(),
            device_public_key: "AAAA".to_string(),
            client_info: OnboardClientInfo {
                agent: "ironclaw".to_string(),
                version: "0.1.0".to_string(),
            },
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["schema_version"], "trace_commons.onboard_request.v1");
        assert_eq!(json["invite_code"], "INV9K3RT5FBQ72JX");
        assert_eq!(json["client_info"]["agent"], "ironclaw");
    }

    #[test]
    fn onboard_response_round_trips_with_optional_label() {
        let json = serde_json::json!({
            "schema_version": "trace_commons.onboard_response.v1",
            "tenant_id": "tenant-zaki-pilot",
            "ingest_url": "https://ingest.example.com",
            "issuer_url": "https://issuer.example.com",
            "audience": "trace-commons-ingest",
            "device_key_id": "sha256:abc123",
        });
        let resp: OnboardResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.tenant_id, "tenant-zaki-pilot");
        assert!(resp.contributor_label.is_none());
        assert!(resp.profile_url.is_none()); // community URLs are optional
    }

    #[test]
    fn onboard_response_parses_community_urls_when_present() {
        let json = serde_json::json!({
            "schema_version": "trace_commons.onboard_response.v1",
            "tenant_id": "t", "ingest_url": "https://i.example",
            "issuer_url": "https://s.example", "audience": "a",
            "device_key_id": "sha256:x",
            "community_url": "https://tracecommons.ai",
            "profile_url": "https://tracecommons.ai/profile",
            "leaderboard_url": "https://tracecommons.ai/leaderboard",
        });
        let resp: OnboardResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.profile_url.as_deref(), Some("https://tracecommons.ai/profile"));
    }

    #[test]
    fn onboard_error_code_parses_known_and_unknown() {
        assert_eq!(
            OnboardErrorCode::parse("InviteNotValid"),
            OnboardErrorCode::InviteNotValid
        );
        assert_eq!(
            OnboardErrorCode::parse("SomethingNew"),
            OnboardErrorCode::Unknown
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ironclaw_reborn_traces onboarding::protocol -- --nocapture`
Expected: compile error (types not defined).

- [ ] **Step 3: Implement the types**

```rust
//! Wire types for the Trace Commons onboarding endpoint.
//! Contract: TraceCommons/trace-commons-server#137. Field names must match exactly.

use serde::{Deserialize, Serialize};

pub const ONBOARD_REQUEST_SCHEMA_VERSION: &str = "trace_commons.onboard_request.v1";

#[derive(Debug, Clone, Serialize)]
pub struct OnboardRequest {
    pub schema_version: &'static str,
    pub invite_code: String,
    /// base64 (standard, padded) of the raw Ed25519 public key bytes.
    pub device_public_key: String,
    pub client_info: OnboardClientInfo,
}

#[derive(Debug, Clone, Serialize)]
pub struct OnboardClientInfo {
    pub agent: String,
    pub version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OnboardResponse {
    pub schema_version: String,
    pub tenant_id: String,
    pub ingest_url: String,
    pub issuer_url: String,
    pub audience: String,
    pub device_key_id: String,
    #[serde(default)]
    pub contributor_label: Option<String>,
    /// Optional browser-surface navigation hints (trace-commons-server#137).
    /// Deployment config, NOT credential material: these never participate in
    /// issuer trust anchoring; non-HTTPS values are dropped, not fatal.
    #[serde(default)]
    pub community_url: Option<String>,
    #[serde(default)]
    pub profile_url: Option<String>,
    #[serde(default)]
    pub leaderboard_url: Option<String>,
}

/// Typed error codes from the onboard endpoint. `InviteNotValid` deliberately
/// covers unknown/consumed/revoked — the server keeps those indistinguishable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardErrorCode {
    InviteNotValid,
    InviteMalformed,
    DeviceKeyMalformed,
    OnboardRateLimited,
    Unknown,
}

impl OnboardErrorCode {
    pub fn parse(code: &str) -> Self {
        match code {
            "InviteNotValid" => Self::InviteNotValid,
            "InviteMalformed" => Self::InviteMalformed,
            "DeviceKeyMalformed" => Self::DeviceKeyMalformed,
            "OnboardRateLimited" => Self::OnboardRateLimited,
            _ => Self::Unknown,
        }
    }
}
```

`mod.rs` for now is just `pub mod protocol;`. Add `pub mod onboarding;` to `lib.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ironclaw_reborn_traces onboarding::protocol`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add crates/ironclaw_reborn_traces/src/onboarding crates/ironclaw_reborn_traces/src/lib.rs
git commit -m "feat(traces): onboarding wire types matching trace-commons-server contract"
```

---

### Task 2: Invite URL parsing (`invite.rs`)

Spec §2.1: canonical `https://<host>/onboard#<code>`, also `?code=` and `code@host` (implies HTTPS, optional port). Origin = scheme+host+port only. HTTPS required except loopback. The invite origin is the trust root.

**Files:**
- Create: `crates/ironclaw_reborn_traces/src/onboarding/invite.rs`
- Modify: `crates/ironclaw_reborn_traces/src/onboarding/mod.rs` (`pub mod invite;`)

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_fragment_form() {
        let p = ParsedInvite::parse("https://issuer.example.com/onboard#INV9K3RT5FBQ72JX").unwrap();
        assert_eq!(p.origin, "https://issuer.example.com");
        assert_eq!(p.code, "INV9K3RT5FBQ72JX");
        assert_eq!(p.issuer_host, "issuer.example.com");
    }

    #[test]
    fn parses_query_form_and_discards_path() {
        let p = ParsedInvite::parse("https://issuer.example.com:8443/anything/else?code=INVAAAA").unwrap();
        assert_eq!(p.origin, "https://issuer.example.com:8443");
        assert_eq!(p.code, "INVAAAA");
    }

    #[test]
    fn parses_code_at_host_form_implying_https() {
        let p = ParsedInvite::parse("INV9K3RT5FBQ72JX@issuer.example.com").unwrap();
        assert_eq!(p.origin, "https://issuer.example.com");
        let p = ParsedInvite::parse("INVAAAA@issuer.example.com:8443").unwrap();
        assert_eq!(p.origin, "https://issuer.example.com:8443");
    }

    #[test]
    fn rejects_non_loopback_http() {
        assert!(matches!(
            ParsedInvite::parse("http://issuer.example.com/onboard#INVAAAA"),
            Err(InviteParseError::InsecureScheme)
        ));
    }

    #[test]
    fn allows_loopback_http_for_dev() {
        let p = ParsedInvite::parse("http://localhost:3917/onboard#INVAAAA").unwrap();
        assert_eq!(p.origin, "http://localhost:3917");
        assert!(ParsedInvite::parse("http://127.0.0.1:3917/onboard#INVAAAA").is_ok());
    }

    #[test]
    fn rejects_empty_or_whitespace_code() {
        assert!(matches!(
            ParsedInvite::parse("https://issuer.example.com/onboard#  "),
            Err(InviteParseError::MissingCode)
        ));
        assert!(ParsedInvite::parse("https://issuer.example.com/onboard").is_err());
    }

    #[test]
    fn onboard_endpoint_is_origin_plus_v1_onboard() {
        let p = ParsedInvite::parse("https://issuer.example.com/onboard#INVAAAA").unwrap();
        assert_eq!(p.onboard_endpoint(), "https://issuer.example.com/v1/onboard");
    }

    #[test]
    fn invite_hash_is_sha256_hex() {
        let p = ParsedInvite::parse("https://h.example/onboard#INVAAAA").unwrap();
        assert_eq!(p.invite_hash().len(), 64);
        assert!(p.invite_hash().chars().all(|c| c.is_ascii_hexdigit()));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p ironclaw_reborn_traces onboarding::invite`
Expected: compile error.

- [ ] **Step 3: Implement**

```rust
//! Invite link parsing. The operator-handed invite link is the trust root
//! (spec §2.1): the invite-derived origin is authoritative for the issuer.

use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InviteParseError {
    #[error("invite link must use https (http allowed for loopback only)")]
    InsecureScheme,
    #[error("invite link is missing an invite code")]
    MissingCode,
    #[error("invite link is malformed: {reason}")]
    Malformed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedInvite {
    /// scheme + host + optional non-default port; no path/query/fragment.
    pub origin: String,
    pub issuer_host: String,
    pub code: String,
}

impl ParsedInvite {
    pub fn parse(raw: &str) -> Result<Self, InviteParseError> {
        let raw = raw.trim();
        // Bare `code@host[:port]` form (implies HTTPS). Only when it doesn't
        // look like a URL at all.
        if !raw.contains("://")
            && let Some((code, host)) = raw.split_once('@')
        {
            return Self::from_parts("https", host, code);
        }
        let url = url::Url::parse(raw)
            .map_err(|e| InviteParseError::Malformed { reason: e.to_string() })?;
        let scheme = url.scheme();
        let host = url.host_str().ok_or_else(|| InviteParseError::Malformed {
            reason: "missing host".to_string(),
        })?;
        let host_port = match url.port() {
            Some(p) => format!("{host}:{p}"),
            None => host.to_string(),
        };
        // Code: fragment wins, then ?code= query param.
        let code = url
            .fragment()
            .map(str::to_string)
            .filter(|f| !f.trim().is_empty())
            .or_else(|| {
                url.query_pairs()
                    .find(|(k, _)| k == "code")
                    .map(|(_, v)| v.into_owned())
            })
            .ok_or(InviteParseError::MissingCode)?;
        Self::from_parts(scheme, &host_port, &code)
    }

    fn from_parts(scheme: &str, host_port: &str, code: &str) -> Result<Self, InviteParseError> {
        let code = code.trim();
        if code.is_empty() {
            return Err(InviteParseError::MissingCode);
        }
        let host_only = host_port.split(':').next().unwrap_or(host_port).to_ascii_lowercase();
        let loopback = host_only == "localhost"
            || host_only.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback());
        if scheme != "https" && !(scheme == "http" && loopback) {
            return Err(InviteParseError::InsecureScheme);
        }
        Ok(Self {
            origin: format!("{scheme}://{host_port}"),
            issuer_host: host_only,
            code: code.to_string(),
        })
    }

    pub fn onboard_endpoint(&self) -> String {
        format!("{}/v1/onboard", self.origin)
    }

    /// SHA-256 hex of the invite code — used for the pending key filename
    /// and matches the server's allowlist subject_hash scheme.
    pub fn invite_hash(&self) -> String {
        hex::encode(Sha256::digest(self.code.as_bytes()))
    }
}
```

Note: `url` crate — check `crates/ironclaw_reborn_traces/Cargo.toml`; `reqwest` re-exports `reqwest::Url` which IS the `url` crate's type and is already used in contribution.rs. If `url` is not a direct dep, use `reqwest::Url` instead of adding a dependency (it is already in the tree via reqwest; do NOT add a new direct dep without checking the workspace table first — if `url` is in the workspace deps table, prefer `url = { workspace = true }`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ironclaw_reborn_traces onboarding::invite`
Expected: 8 passed.

- [ ] **Step 5: Commit**

```bash
cargo fmt && git add -A crates/ironclaw_reborn_traces && git commit -m "feat(traces): invite URL parsing with origin trust anchoring"
```

---

### Task 3: Device keypair lifecycle (`device_key.rs`)

Spec §2.2: Ed25519 per (scope, tenant); staged at `device_keys/pending/<invite-hash>.json` (0600) before the network call; atomically promoted to `device_keys/<tenant-hash>.json`; pending file deleted on terminal failure; reuse on retry from either path. Plus the self-signed workload JWT (spec §2.4) — manual base64url construction signed with `ed25519-dalek` (avoids PKCS8 plumbing that `jsonwebtoken`'s `EncodingKey` would require).

**Files:**
- Create: `crates/ironclaw_reborn_traces/src/onboarding/device_key.rs`
- Modify: `crates/ironclaw_reborn_traces/src/onboarding/mod.rs` (`pub mod device_key;`)
- Modify: `crates/ironclaw_reborn_traces/Cargo.toml` — add `ed25519-dalek = { version = "2.2.0", features = ["std"] }` and `rand = "0.8"` to `[dependencies]`, matching the versions in the root package's `[dependencies]` (NOTE: this repo has NO `[workspace.dependencies]` table — do not write `{ workspace = true }`, it will not resolve; crates use literal versions, like the existing `base64 = "0.22.1"`). ed25519-dalek 2.x's `SigningKey::generate` needs `rand_core::OsRng`; ed25519-dalek 2.2 pairs with rand 0.8 via `rand_core` — there is no existing ed25519 keygen in this tree to copy, so this pairing is the idiom to use. Also add `tempfile = "3"` to `[dev-dependencies]` (currently the only dev-dep is `axum`; Tasks 3, 5 and 6 all use `tempfile::tempdir()`).

Key derivation rule (must match server, see #137): `device_key_id = "sha256:" + lowercase hex of SHA-256 of the raw 32-byte public key`. Tenant hash for the promoted filename: a private full-width helper here — `hex::encode(Sha256::digest(tenant_id))` (64 hex chars). Do NOT reuse the existing `scope_hash()` (contribution.rs:7701): it truncates to 16 bytes / 32 hex chars, which is not what we want for the tenant filename.

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn generates_and_stages_pending_keypair_with_0600() {
        let dir = tmp_dir();
        let kp = DeviceKeypair::load_or_generate_pending(dir.path(), "abc123hash").unwrap();
        let pending = dir.path().join("device_keys/pending/abc123hash.json");
        assert!(pending.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&pending).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(kp.device_key_id.starts_with("sha256:"));
    }

    #[test]
    fn reloads_same_pending_keypair_on_retry() {
        let dir = tmp_dir();
        let a = DeviceKeypair::load_or_generate_pending(dir.path(), "h1").unwrap();
        let b = DeviceKeypair::load_or_generate_pending(dir.path(), "h1").unwrap();
        assert_eq!(a.device_key_id, b.device_key_id);
    }

    #[test]
    fn promote_moves_pending_to_tenant_path_and_records_tenant() {
        let dir = tmp_dir();
        let kp = DeviceKeypair::load_or_generate_pending(dir.path(), "h1").unwrap();
        let promoted = kp.promote(dir.path(), "h1", "tenant-a").unwrap();
        assert!(!dir.path().join("device_keys/pending/h1.json").exists());
        let tenant_file = dir.path().join(format!("device_keys/{}.json", tenant_hash("tenant-a")));
        assert!(tenant_file.exists());
        assert_eq!(promoted.tenant_id.as_deref(), Some("tenant-a"));
    }

    #[test]
    fn load_for_tenant_finds_promoted_key() {
        let dir = tmp_dir();
        let kp = DeviceKeypair::load_or_generate_pending(dir.path(), "h1").unwrap();
        let promoted = kp.promote(dir.path(), "h1", "tenant-a").unwrap();
        let loaded = DeviceKeypair::load_for_tenant(dir.path(), "tenant-a").unwrap().unwrap();
        assert_eq!(loaded.device_key_id, promoted.device_key_id);
    }

    #[test]
    fn discard_pending_removes_file() {
        let dir = tmp_dir();
        DeviceKeypair::load_or_generate_pending(dir.path(), "h1").unwrap();
        DeviceKeypair::discard_pending(dir.path(), "h1").unwrap();
        assert!(!dir.path().join("device_keys/pending/h1.json").exists());
    }

    #[test]
    fn self_signed_workload_jwt_has_correct_shape_and_verifies() {
        let dir = tmp_dir();
        let kp = DeviceKeypair::load_or_generate_pending(dir.path(), "h1").unwrap();
        let kp = kp.promote(dir.path(), "h1", "tenant-a").unwrap();
        let jwt = kp.sign_workload_jwt("trace-commons-ingest").unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);

        let header = jsonwebtoken::decode_header(&jwt).unwrap();
        assert_eq!(header.alg, jsonwebtoken::Algorithm::EdDSA);
        assert_eq!(header.kid.as_deref(), Some(kp.device_key_id.as_str()));

        use base64::Engine as _;
        let payload: serde_json::Value = serde_json::from_slice(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1]).unwrap(),
        ).unwrap();
        assert_eq!(payload["tenant_id"], "tenant-a");
        assert_eq!(payload["aud"], "trace-commons-ingest");
        let iat = payload["iat"].as_i64().unwrap();
        let exp = payload["exp"].as_i64().unwrap();
        assert_eq!(exp - iat, 60);

        // Signature verifies against the public key with ed25519-dalek.
        use ed25519_dalek::Verifier as _;
        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let sig_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
        let sig = ed25519_dalek::Signature::from_slice(&sig_bytes).unwrap();
        kp.verifying_key().unwrap().verify(signing_input.as_bytes(), &sig).unwrap();
    }

    #[test]
    fn key_file_never_contains_unencoded_private_key_field_names_in_logs_shape() {
        // The Debug impl must not leak the private key.
        let dir = tmp_dir();
        let kp = DeviceKeypair::load_or_generate_pending(dir.path(), "h1").unwrap();
        let dbg = format!("{kp:?}");
        assert!(!dbg.contains(&kp.private_key_b64_for_test()));
    }
}
```

(`tempfile` was added to `[dev-dependencies]` in the Files step above.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p ironclaw_reborn_traces onboarding::device_key`
Expected: compile error.

- [ ] **Step 3: Implement**

Core shape (complete the obvious helpers; keep `Debug` manual to redact the private key):

```rust
//! Per-(scope, tenant) Ed25519 device keypairs (spec §2.2) and self-signed
//! workload JWTs (spec §2.4). Private keys never leave the machine.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD as B64, URL_SAFE_NO_PAD as B64URL};
use ed25519_dalek::{Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const WORKLOAD_JWT_TTL_SECS: i64 = 60;

#[derive(Debug, Error)]
pub enum DeviceKeyError {
    #[error("device key io error: {reason}")]
    Io { reason: String },
    #[error("device key file is malformed: {reason}")]
    Malformed { reason: String },
}

#[derive(Serialize, Deserialize)]
struct DeviceKeyFile {
    private_key: String, // base64 of 32-byte Ed25519 secret
    public_key: String,  // base64 of 32-byte public key
    device_key_id: String,
    #[serde(default)]
    tenant_id: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
}

pub struct DeviceKeypair {
    signing_key: SigningKey,
    pub device_key_id: String,
    pub public_key_b64: String,
    pub tenant_id: Option<String>,
}

pub(crate) fn tenant_hash(tenant_id: &str) -> String {
    hex::encode(Sha256::digest(tenant_id.as_bytes()))
}

fn device_key_id_for(public: &VerifyingKey) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(public.as_bytes())))
}

impl DeviceKeypair {
    /// Stage (or reload) the pending keypair for an invite, persisting it
    /// BEFORE any network call (spec §2.2 retry safety).
    pub fn load_or_generate_pending(base: &Path, invite_hash: &str) -> Result<Self, DeviceKeyError>;

    /// Atomically promote pending → device_keys/<tenant-hash>.json, recording the tenant.
    pub fn promote(self, base: &Path, invite_hash: &str, tenant_id: &str) -> Result<Self, DeviceKeyError>;

    /// Load the promoted key for a tenant, if any.
    pub fn load_for_tenant(base: &Path, tenant_id: &str) -> Result<Option<Self>, DeviceKeyError>;

    /// Delete a pending key after a terminal onboard failure (spec §2.2 hygiene).
    pub fn discard_pending(base: &Path, invite_hash: &str) -> Result<(), DeviceKeyError>;

    pub fn verifying_key(&self) -> Result<VerifyingKey, DeviceKeyError>;

    /// Self-signed workload JWT: header {alg: EdDSA, typ: JWT, kid}, claims
    /// {tenant_id, aud, iat, exp = iat + 60}. Manual base64url construction —
    /// jsonwebtoken's EncodingKey requires PKCS8 PEM, which is more plumbing
    /// than signing the two base64url segments with dalek directly.
    pub fn sign_workload_jwt(&self, audience: &str) -> Result<String, DeviceKeyError> {
        let tenant_id = self.tenant_id.as_deref().ok_or_else(|| DeviceKeyError::Malformed {
            reason: "device key has no tenant binding".to_string(),
        })?;
        let header = serde_json::json!({
            "alg": "EdDSA", "typ": "JWT", "kid": self.device_key_id,
        });
        let iat = chrono::Utc::now().timestamp();
        let claims = serde_json::json!({
            "tenant_id": tenant_id, "aud": audience,
            "iat": iat, "exp": iat + WORKLOAD_JWT_TTL_SECS,
        });
        let signing_input = format!(
            "{}.{}",
            B64URL.encode(serde_json::to_vec(&header).map_err(|e| DeviceKeyError::Malformed { reason: e.to_string() })?),
            B64URL.encode(serde_json::to_vec(&claims).map_err(|e| DeviceKeyError::Malformed { reason: e.to_string() })?),
        );
        let sig = self.signing_key.sign(signing_input.as_bytes());
        Ok(format!("{signing_input}.{}", B64URL.encode(sig.to_bytes())))
    }
}
```

Implementation notes for the storage methods:
- Files live under `<base>/device_keys/` where `base` is the scoped trace-contribution dir (passed in by the caller — this module never computes scope paths itself; keeps it unit-testable with tempdirs).
- Write with `std::fs::create_dir_all` + write to a `.tmp` sibling + `set_permissions(0o600)` (cfg(unix)) + `std::fs::rename` for atomicity. Promotion is also a rename (same filesystem) after rewriting the JSON with `tenant_id` set.
- `load_or_generate_pending`: if the pending file exists, parse and return it; else `SigningKey::generate(&mut rand::rngs::OsRng)` (adjust to the workspace's rand version idiom), persist, return.
- Manual `impl std::fmt::Debug for DeviceKeypair` that omits the signing key.
- `private_key_b64_for_test()` behind `#[cfg(test)]`.

- [ ] **Step 4: Run tests, verify pass**

Run: `cargo test -p ironclaw_reborn_traces onboarding::device_key`
Expected: 7 passed.

- [ ] **Step 5: Commit**

```bash
cargo fmt && git add -A crates/ironclaw_reborn_traces && git commit -m "feat(traces): device keypair lifecycle with pending staging and self-signed workload JWTs"
```

---

### Task 4: Policy fields — `auth_mode` + `device_key_id`

Spec §2.3. Old policy files must deserialize as `WorkloadTokenEnv` (serde default).

**Files:**
- Modify: `crates/ironclaw_reborn_traces/src/contribution.rs` (policy struct ~line 633, Default impl ~line 726)

- [ ] **Step 1: Write failing tests** (in contribution.rs's existing `#[cfg(test)]` module, or a new small one near the policy code)

```rust
#[test]
fn legacy_policy_json_defaults_to_workload_token_env_auth() {
    let legacy = serde_json::json!({
        "enabled": true,
        "bearer_token_env": "IRONCLAW_TRACE_SUBMIT_TOKEN",
        "include_message_text": false,
        "include_tool_payloads": false,
        "auto_submit_failed_traces": false,
        "auto_submit_high_value_traces": false,
        "require_manual_approval_when_pii_detected": true,
        "min_submission_score": 0.35,
        "credit_notice_interval_hours": 24,
        "default_scope": "debugging-evaluation"
    });
    let policy: StandingTraceContributionPolicy = serde_json::from_value(legacy).unwrap();
    assert_eq!(policy.auth_mode, TraceUploadAuthMode::WorkloadTokenEnv);
    assert!(policy.device_key_id.is_none());
}

#[test]
fn device_key_policy_round_trips() {
    let mut policy = StandingTraceContributionPolicy::default();
    policy.auth_mode = TraceUploadAuthMode::DeviceKey;
    policy.device_key_id = Some("sha256:abc".to_string());
    let json = serde_json::to_value(&policy).unwrap();
    let back: StandingTraceContributionPolicy = serde_json::from_value(json).unwrap();
    assert_eq!(back.auth_mode, TraceUploadAuthMode::DeviceKey);
    assert_eq!(back.device_key_id.as_deref(), Some("sha256:abc"));
}
```

(Adjust the legacy JSON literal to include every non-default-able field the real struct requires — copy the exact required-field set from the struct definition; the point of the test is only that the two NEW fields default correctly.)

- [ ] **Step 2: Run to verify failure** — `cargo test -p ironclaw_reborn_traces legacy_policy_json` — compile error.

- [ ] **Step 3: Implement**

Add near the policy struct:

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceUploadAuthMode {
    /// Operator-minted workload token read from env (legacy/back-compat path).
    #[default]
    WorkloadTokenEnv,
    /// Self-signed workload JWTs using the local device key (agent onboarding path).
    DeviceKey,
}
```

Add to `StandingTraceContributionPolicy`:

```rust
#[serde(default)]
pub auth_mode: TraceUploadAuthMode,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub device_key_id: Option<String>,
```

And to its `Default` impl: `auth_mode: TraceUploadAuthMode::default(), device_key_id: None,`. Fix any struct-literal construction sites that now miss fields: the CLI opt-in builder constructs the policy as a struct literal at `ironclaw_reborn_cli/src/commands/traces/mod.rs:440`, and there may be more — grep `StandingTraceContributionPolicy {` (with the space-brace) across all of `crates/ironclaw_reborn_cli/src/commands/traces/` (`mod.rs`, `contributor.rs`, `shared.rs`, `tests.rs`) and `crates/ironclaw_reborn_traces/`; `cargo check -p ironclaw_reborn_traces -p ironclaw_reborn_cli` is the backstop. Add the two defaults explicitly at each site.

- [ ] **Step 4: Run** — `cargo test -p ironclaw_reborn_traces legacy_policy_json device_key_policy && cargo check -p ironclaw_reborn_cli` — pass.

- [ ] **Step 5: Commit** — `cargo fmt && git add -A crates && git commit -m "feat(traces): auth_mode and device_key_id policy fields with legacy-compatible defaults"`

---

### Task 5: `onboard()` orchestration (`mod.rs`) with mock issuer

Spec §2.3 sequence: parse → stage keypair → POST → verify response `issuer_url` origin == invite origin → promote keypair → write policy. Trust anchoring per §2.1: `upload_token_issuer_url` and allowed-hosts are seeded from the INVITE origin; `ingest_url` from the response must be HTTPS (loopback http allowed in tests via the same rule as invite parsing). Terminal `InviteNotValid`/`InviteMalformed`/`DeviceKeyMalformed` → discard pending key.

**Files:**
- Modify: `crates/ironclaw_reborn_traces/src/onboarding/mod.rs`

- [ ] **Step 1: Write failing tests** — use an `axum` loopback mock issuer (pattern: `reborn_webui_ingress/tests/oidc_e2e.rs`). In `mod.rs`'s test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::post};
    use std::sync::{Arc, Mutex};

    struct MockIssuer {
        addr: std::net::SocketAddr,
        requests: Arc<Mutex<Vec<serde_json::Value>>>,
        _handle: tokio::task::JoinHandle<()>,
    }

    async fn spawn_mock_issuer(response: serde_json::Value, status: axum::http::StatusCode) -> MockIssuer {
        let requests: Arc<Mutex<Vec<serde_json::Value>>> = Arc::default();
        let reqs = requests.clone();
        let app = Router::new().route(
            "/v1/onboard",
            post(move |Json(body): Json<serde_json::Value>| {
                let reqs = reqs.clone();
                let response = response.clone();
                async move {
                    reqs.lock().unwrap().push(body);
                    (status, Json(response))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        MockIssuer { addr, requests, _handle: handle }
    }

    fn ok_response(origin: &str) -> serde_json::Value {
        serde_json::json!({
            "schema_version": "trace_commons.onboard_response.v1",
            "tenant_id": "tenant-a",
            "ingest_url": "https://ingest.example.com",
            "issuer_url": origin,
            "audience": "trace-commons-ingest",
            "device_key_id": "sha256:server-side-recomputed",
        })
    }

    #[tokio::test]
    async fn successful_onboard_writes_policy_and_promotes_key() {
        let dir = tempfile::tempdir().unwrap();
        // spawn first to know the origin, then make the response echo it
        let issuer = spawn_mock_issuer(serde_json::Value::Null, axum::http::StatusCode::OK).await;
        // (restructure: build response after addr known — see implementation note)
        let origin = format!("http://127.0.0.1:{}", issuer.addr.port());
        let issuer = spawn_mock_issuer(ok_response(&origin), axum::http::StatusCode::OK).await;
        let origin = format!("http://127.0.0.1:{}", issuer.addr.port());
        // NOTE: the double-spawn above is illustrative; implement a builder
        // where the response issuer_url is computed from the bound addr.

        let invite = format!("{origin}/onboard#INVAAAA");
        let consents = OnboardConsents { include_message_text: true, include_tool_payloads: false };
        let outcome = onboard_at_dir(dir.path(), &invite, consents).await.unwrap();

        assert_eq!(outcome.tenant_id, "tenant-a");
        // policy written
        let policy: StandingTraceContributionPolicy = serde_json::from_slice(
            &std::fs::read(dir.path().join("policy.json")).unwrap()).unwrap();
        assert!(policy.enabled);
        assert_eq!(policy.auth_mode, TraceUploadAuthMode::DeviceKey);
        assert_eq!(policy.upload_token_issuer_url.as_deref(), Some(origin.as_str()));
        assert_eq!(policy.ingestion_endpoint.as_deref(), Some("https://ingest.example.com"));
        assert!(policy.include_message_text);
        assert!(!policy.include_tool_payloads);
        // key promoted, pending gone
        assert!(dir.path().join(format!("device_keys/{}.json", device_key::tenant_hash("tenant-a"))).exists());
        assert!(!dir.path().join("device_keys/pending").join(format!("{}.json", ParsedInvite::parse(&invite).unwrap().invite_hash())).exists());
        // request carried the pubkey
        let sent = issuer.requests.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(sent[0]["device_public_key"].as_str().is_some());
    }

    #[tokio::test]
    async fn issuer_url_mismatch_rejects_onboard() {
        // response issuer_url is a different origin than the invite
        let issuer = spawn_mock_issuer(ok_response("https://evil.example.com"), axum::http::StatusCode::OK).await;
        let origin = format!("http://127.0.0.1:{}", issuer.addr.port());
        let dir = tempfile::tempdir().unwrap();
        let err = onboard_at_dir(dir.path(), &format!("{origin}/onboard#INVAAAA"),
            OnboardConsents::default()).await.unwrap_err();
        assert!(matches!(err, OnboardError::IssuerOriginMismatch { .. }));
        // no policy written
        assert!(!dir.path().join("policy.json").exists());
    }

    #[tokio::test]
    async fn invite_not_valid_discards_pending_key() {
        let issuer = spawn_mock_issuer(
            serde_json::json!({"error": "InviteNotValid"}),
            axum::http::StatusCode::FORBIDDEN,
        ).await;
        let origin = format!("http://127.0.0.1:{}", issuer.addr.port());
        let dir = tempfile::tempdir().unwrap();
        let err = onboard_at_dir(dir.path(), &format!("{origin}/onboard#INVAAAA"),
            OnboardConsents::default()).await.unwrap_err();
        assert!(matches!(err, OnboardError::InviteRejected(OnboardErrorCode::InviteNotValid)));
        assert!(!dir.path().join("device_keys/pending").exists()
            || std::fs::read_dir(dir.path().join("device_keys/pending")).unwrap().next().is_none());
    }

    #[tokio::test]
    async fn community_urls_pass_through_when_https_and_drop_when_not() {
        // ok_response + {"profile_url": "https://tracecommons.ai/profile",
        //                "leaderboard_url": "http://insecure.example"}
        // → outcome.profile_url = Some(...), outcome.leaderboard_url = None,
        //   and the onboard still succeeds (nav hints are never fatal).
        // (Implement with the same mock-issuer builder.)
    }

    #[tokio::test]
    async fn non_https_ingest_url_rejected() {
        let issuer = spawn_mock_issuer(serde_json::Value::Null, axum::http::StatusCode::OK).await;
        let origin = format!("http://127.0.0.1:{}", issuer.addr.port());
        let mut resp = ok_response(&origin);
        resp["ingest_url"] = serde_json::json!("http://ingest.example.com");
        let issuer = spawn_mock_issuer(resp, axum::http::StatusCode::OK).await;
        let origin = format!("http://127.0.0.1:{}", issuer.addr.port());
        let dir = tempfile::tempdir().unwrap();
        let err = onboard_at_dir(dir.path(), &format!("{origin}/onboard#INVAAAA"),
            OnboardConsents::default()).await.unwrap_err();
        assert!(matches!(err, OnboardError::InsecureIngestUrl { .. }));
    }
}
```

(Clean up the illustrative double-spawn when implementing: make `spawn_mock_issuer` take a closure `FnOnce(SocketAddr) -> serde_json::Value`.)

- [ ] **Step 2: Run to verify failure** — `cargo test -p ironclaw_reborn_traces onboarding::tests` — compile error.

- [ ] **Step 3: Implement**

```rust
pub mod device_key;
pub mod invite;
pub mod protocol;

use std::path::Path;

use thiserror::Error;

pub use device_key::{DeviceKeyError, DeviceKeypair};
pub use invite::{InviteParseError, ParsedInvite};
pub use protocol::{OnboardErrorCode, OnboardRequest, OnboardResponse};

use crate::contribution::{
    ConsentScope, StandingTraceContributionPolicy, TraceUploadAuthMode,
    trace_contribution_dir_for_scope, write_trace_policy_for_scope,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct OnboardConsents {
    pub include_message_text: bool,
    pub include_tool_payloads: bool,
}

#[derive(Debug, Clone)]
pub struct OnboardOutcome {
    pub tenant_id: String,
    pub ingest_url: String,
    pub issuer_url: String,
    pub device_key_id: String,
    pub contributor_label: Option<String>,
    /// Browser navigation hints from the server (trace-commons-server#137).
    /// Sanitized: each is dropped (None) unless HTTPS — never fatal.
    pub community_url: Option<String>,
    pub profile_url: Option<String>,
    pub leaderboard_url: Option<String>,
}

#[derive(Debug, Error)]
pub enum OnboardError {
    #[error(transparent)]
    InvalidInvite(#[from] InviteParseError),
    #[error(transparent)]
    DeviceKey(#[from] DeviceKeyError),
    #[error("the onboarding server rejected the invite: {0:?}")]
    InviteRejected(OnboardErrorCode),
    #[error("onboarding response issuer_url ({response}) does not match the invite origin ({invite}); refusing")]
    IssuerOriginMismatch { invite: String, response: String },
    #[error("onboarding response ingest_url is not https: {url}")]
    InsecureIngestUrl { url: String },
    #[error("could not reach the onboarding server: {reason}")]
    Network { reason: String },
    #[error("onboarding response was malformed: {reason}")]
    MalformedResponse { reason: String },
    #[error("failed to persist onboarding state: {reason}")]
    Persist { reason: String },
}

/// Public entry: resolves the scoped trace-contribution dir, then runs the flow.
pub async fn onboard(
    scope: &str,
    invite_url: &str,
    consents: OnboardConsents,
) -> Result<OnboardOutcome, OnboardError> {
    let dir = trace_contribution_dir_for_scope(Some(scope));
    let outcome = onboard_at_dir(&dir, invite_url, consents).await?;
    // Also write the scoped policy through the canonical helper so any
    // side-channel state stays consistent with the CLI path.
    Ok(outcome)
}

/// Dir-parameterized core (unit-testable with tempdirs).
pub async fn onboard_at_dir(
    dir: &Path,
    invite_url: &str,
    consents: OnboardConsents,
) -> Result<OnboardOutcome, OnboardError> {
    let invite = ParsedInvite::parse(invite_url)?;
    let pending = DeviceKeypair::load_or_generate_pending(dir, &invite.invite_hash())?;

    let response = post_onboard(&invite, &pending).await;
    let response = match response {
        Ok(r) => r,
        Err(err @ OnboardError::InviteRejected(
            OnboardErrorCode::InviteNotValid
            | OnboardErrorCode::InviteMalformed
            | OnboardErrorCode::DeviceKeyMalformed,
        )) => {
            // Terminal failure: the staged key was never registered (spec §2.2 hygiene).
            DeviceKeypair::discard_pending(dir, &invite.invite_hash())?;
            return Err(err);
        }
        Err(err) => return Err(err), // transient: keep pending key for retry
    };

    // Trust anchoring (spec §2.1): invite origin is authoritative.
    if normalized_origin(&response.issuer_url) != invite.origin {
        return Err(OnboardError::IssuerOriginMismatch {
            invite: invite.origin.clone(),
            response: response.issuer_url.clone(),
        });
    }
    ensure_https_or_loopback(&response.ingest_url)
        .map_err(|_| OnboardError::InsecureIngestUrl { url: response.ingest_url.clone() })?;

    let key = pending.promote(dir, &invite.invite_hash(), &response.tenant_id)?;

    let mut policy = StandingTraceContributionPolicy::default();
    policy.enabled = true;
    policy.auth_mode = TraceUploadAuthMode::DeviceKey;
    policy.device_key_id = Some(key.device_key_id.clone());
    policy.ingestion_endpoint = Some(response.ingest_url.clone());
    policy.upload_token_issuer_url = Some(invite.origin.clone()); // invite-derived, not response
    policy.upload_token_issuer_allowed_hosts = std::iter::once(invite.issuer_host.clone()).collect();
    policy.upload_token_audience = Some(response.audience.clone());
    policy.upload_token_tenant_id = Some(response.tenant_id.clone());
    policy.include_message_text = consents.include_message_text;
    policy.include_tool_payloads = consents.include_tool_payloads;
    policy.default_scope = ConsentScope::DebuggingEvaluation; // pilot default (verify variant name)
    write_policy_at_dir(dir, &policy)?;

    // Community URLs are navigation hints only: sanitize (HTTPS or drop),
    // never participate in trust anchoring, never fail the onboard.
    let sanitize_nav = |u: Option<String>| u.filter(|u| u.starts_with("https://"));

    Ok(OnboardOutcome {
        tenant_id: response.tenant_id,
        ingest_url: response.ingest_url,
        issuer_url: invite.origin,
        device_key_id: key.device_key_id,
        contributor_label: response.contributor_label,
        community_url: sanitize_nav(response.community_url),
        profile_url: sanitize_nav(response.profile_url),
        leaderboard_url: sanitize_nav(response.leaderboard_url),
    })
}
```

Implementation notes:
- `post_onboard`: `reqwest::Client` with the same builder hygiene as `fetch_trace_upload_claim_from_issuer` (timeout, no redirects, explicit user agent `ironclaw-trace-commons-onboard/0.1`). 4xx with a JSON `{"error": "<code>"}` body → `OnboardError::InviteRejected(OnboardErrorCode::parse(code))`; map missing/unparsable code to `OnboardErrorCode::Unknown` which is treated as transient (keep the pending key). Cap response body read (follow the existing 64 KB cap convention).
- `onboard()` vs `onboard_at_dir()`: the spec's per-scope behavior comes via `trace_contribution_dir_for_scope`; `write_policy_at_dir` writes `<dir>/policy.json` the same way `write_trace_policy_for_scope` does (reuse the existing `write_json_file` helper if visible; otherwise a small local helper with the same semantics). Important: check whether `write_trace_policy_for_scope` does anything beyond the path write (it doesn't, per current code at contribution.rs:4018) — if that changes, route through it.
- `normalized_origin`: parse with `Url`, rebuild `scheme://host[:port]` (drop default ports consistently — use `Url::port()` which already returns `None` for defaults).
- `ensure_https_or_loopback`: same rule as invite parsing — extract the helper into `invite.rs` and reuse.

- [ ] **Step 4: Run tests** — `cargo test -p ironclaw_reborn_traces onboarding` — all pass (Tasks 1-3 tests + 4 new).

- [ ] **Step 5: Commit** — `cargo fmt && git add -A crates/ironclaw_reborn_traces && git commit -m "feat(traces): onboard() orchestration with trust anchoring and retry-safe key staging"`

---

### Task 6: DeviceKey branch in upload-claim refresh

Spec §2.4: when `auth_mode = DeviceKey`, self-sign the workload JWT with the device key instead of reading the workload-token env var.

**Files:**
- Modify: `crates/ironclaw_reborn_traces/src/contribution.rs` — `fetch_trace_upload_claim_from_issuer`, specifically the bearer-auth section at ~lines 4883-4893.

- [ ] **Step 1: Write failing test**

The existing tests use the `TraceUploadCredentialProvider` trait with fakes; the env-read branch is inside `fetch_trace_upload_claim_from_issuer`. Test at the same level the existing issuer-fetch tests do (find them: search `fetch_trace_upload_claim` in the `#[cfg(test)]` module; if the fetch path is only integration-tested, add a focused unit test on a new extracted helper instead):

Extract a helper so the branch is testable without HTTP:

```rust
/// Returns the bearer credential to present to the upload-claim issuer.
async fn issuer_request_bearer(
    policy: &StandingTraceContributionPolicy,
    scope_dir: &Path,
) -> anyhow::Result<Option<String>>
```

Test:

```rust
#[tokio::test]
async fn device_key_auth_mode_self_signs_workload_jwt() {
    let dir = tempfile::tempdir().unwrap();
    let kp = crate::onboarding::DeviceKeypair::load_or_generate_pending(dir.path(), "h").unwrap();
    let kp = kp.promote(dir.path(), "h", "tenant-a").unwrap();

    let mut policy = StandingTraceContributionPolicy::default();
    policy.auth_mode = TraceUploadAuthMode::DeviceKey;
    policy.device_key_id = Some(kp.device_key_id.clone());
    policy.upload_token_tenant_id = Some("tenant-a".to_string());
    policy.upload_token_audience = Some("trace-commons-ingest".to_string());

    let bearer = issuer_request_bearer(&policy, dir.path()).await.unwrap().unwrap();
    let header = jsonwebtoken::decode_header(&bearer).unwrap();
    assert_eq!(header.alg, jsonwebtoken::Algorithm::EdDSA);
    assert_eq!(header.kid.as_deref(), Some(kp.device_key_id.as_str()));
}

#[tokio::test]
async fn device_key_auth_mode_without_local_key_errors_clearly() {
    let dir = tempfile::tempdir().unwrap();
    let mut policy = StandingTraceContributionPolicy::default();
    policy.auth_mode = TraceUploadAuthMode::DeviceKey;
    policy.upload_token_tenant_id = Some("tenant-a".to_string());
    policy.upload_token_audience = Some("aud".to_string());
    let err = issuer_request_bearer(&policy, dir.path()).await.unwrap_err();
    assert!(err.to_string().contains("device key"));
}

#[tokio::test]
async fn workload_token_env_mode_unchanged() {
    // existing behavior: env var read — assert the helper returns the env value
    // (use a uniquely-named env var; tests setting env vars must be serial or unique-named)
    let mut policy = StandingTraceContributionPolicy::default();
    policy.upload_token_workload_token_env = Some("TEST_ONBOARD_WL_TOKEN_UNIQUE".to_string());
    std::env::set_var("TEST_ONBOARD_WL_TOKEN_UNIQUE", "tok");
    let dir = tempfile::tempdir().unwrap();
    let bearer = issuer_request_bearer(&policy, dir.path()).await.unwrap();
    assert_eq!(bearer.as_deref(), Some("tok"));
    std::env::remove_var("TEST_ONBOARD_WL_TOKEN_UNIQUE");
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p ironclaw_reborn_traces device_key_auth_mode` — compile error.

- [ ] **Step 3: Implement**

Extract the current env-read block (contribution.rs:4883-4893) into `issuer_request_bearer` and add the branch:

```rust
async fn issuer_request_bearer(
    policy: &StandingTraceContributionPolicy,
    scope_dir: &Path,
) -> anyhow::Result<Option<String>> {
    match policy.auth_mode {
        TraceUploadAuthMode::DeviceKey => {
            let tenant = policy.upload_token_tenant_id.as_deref().ok_or_else(|| {
                anyhow::anyhow!("device-key auth requires upload_token_tenant_id in the trace policy")
            })?;
            let audience = policy.upload_token_audience.as_deref().ok_or_else(|| {
                anyhow::anyhow!("device-key auth requires upload_token_audience in the trace policy")
            })?;
            let key = crate::onboarding::DeviceKeypair::load_for_tenant(scope_dir, tenant)
                .map_err(|e| anyhow::anyhow!("failed to load device key: {e}"))?
                .ok_or_else(|| anyhow::anyhow!(
                    "trace policy is in device-key auth mode but no device key exists for tenant {tenant}; re-run onboarding"
                ))?;
            Ok(Some(key.sign_workload_jwt(audience)?))
        }
        TraceUploadAuthMode::WorkloadTokenEnv => {
            // existing env-read logic moved here verbatim
            /* ... */
        }
    }
}
```

**Scope threading — this is a real refactor, budget for it.** The claim-fetch path does NOT currently know the user scope: `TraceUploadClaimContext` (contribution.rs:4331) carries only `trace_id`/`submission_id`/`consent_scopes`/`allowed_uses`, the `TraceUploadCredentialProvider::bearer_token` trait method (~contribution.rs:4602-4615) takes `(policy, context, force_refresh)` with no scope, and the claim cache key is built from policy+context. To give `issuer_request_bearer` a real `scope_dir` you must thread the scope down explicitly:

1. Add `scope_dir: PathBuf` (or `scope: Option<String>` resolved to a dir at the leaf) to `TraceUploadClaimContext` — preferred over changing the trait signature, since the context already flows everywhere it's needed. Construction happens via the helper constructors `TraceUploadClaimContext::for_envelope`/`::for_status_sync`/`::for_submission_id` (contribution.rs:4339/4348/4357) — extend those, not a struct literal. Depth caveat: the three production callers (`submit_..._with_credential_provider` ~5187, the status-sync fetch ~5644, revoke ~6107) do NOT currently take a scope parameter, so threading extends one layer further up to their callers (the CLI commands have `--user-scope`/default; the autonomous submit path has the runtime scope). The TDD threading test below won't compile until the scope actually reaches the leaf — let the compiler walk you up the chain.
2. Pass `context.scope_dir` into `issuer_request_bearer` from `fetch_trace_upload_claim_from_issuer`.
3. Update every `TraceUploadCredentialProvider` impl and test fake (e.g. `RefreshingTestUploadCredentialProvider`, contribution.rs ~7867) for the new context field.
4. Write a focused failing test FIRST for the threading itself: construct a `TraceUploadClaimContext` with a tempdir `scope_dir`, a `DeviceKey` policy, a promoted key in that dir, and assert `issuer_request_bearer` finds the key (this is the `device_key_auth_mode_self_signs_workload_jwt` test above — just be aware making it compile requires steps 1-3, not a lookup).

Also: in `DeviceKey` mode, do NOT include `invite_code` in the `TraceUploadClaimIssuerRequest` (spec: the registered key is the post-invite credential) — gate the existing `invite_code: policy.upload_token_invite_code.clone()` line on `auth_mode == WorkloadTokenEnv`.

- [ ] **Step 4: Run the full crate tests** — `cargo test -p ironclaw_reborn_traces` — all pass (regression check on the env path included).

- [ ] **Step 5: Commit** — `cargo fmt && git add -A crates/ironclaw_reborn_traces && git commit -m "feat(traces): device-key self-signed workload JWT branch in upload-claim refresh"`

---

### Task 7: Engine tools — `trace_commons.onboard` + `trace_commons.status`

First-party capability pattern in `crates/ironclaw_host_runtime/src/first_party_tools/`. **Dispatch model (do not assume one-handler-per-file):** there is a single `impl FirstPartyCapabilityHandler for BuiltinFirstPartyTools` in `mod.rs` (~line 144) that routes by `match request.capability_id.as_str()` (~line 152); files like `echo.rs`/`http.rs` are helper modules exposing a `manifest()` and a dispatch function the central match calls. `builtin_first_party_handlers()` (mod.rs:88-106) registers the *same* `Arc<BuiltinFirstPartyTools>` under each capability ID. So this task = new `trace_commons.rs` module with free functions + two match arms in the central dispatch + two `.with_handler(CapabilityId::new(...)?, handler.clone())` lines.

The tool layer is thin: validate input, call `ironclaw_reborn_traces::onboarding::onboard()` / read policy+queue state, shape JSON output. **No key material in any output.** Scope source: `FirstPartyCapabilityRequest` carries `scope: ResourceScope` with a `user_id: UserId` (see `ironclaw_host_api/src/resource.rs`) — use `request.scope.user_id.as_str()` as the trace scope fed to `trace_contribution_dir_for_scope(Some(...))`/`onboard(scope, ...)`.

**Files:**
- Create: `crates/ironclaw_host_runtime/src/first_party_tools/trace_commons.rs`
- Modify: `crates/ironclaw_host_runtime/src/first_party_tools/mod.rs` (module decl + two registrations)
- Modify: `crates/ironclaw_host_runtime/Cargo.toml` (add `ironclaw_reborn_traces` dep if absent — check first; if adding, `{ path = "../ironclaw_reborn_traces" }` following neighboring entries)

Before writing code, read `first_party_tools/echo.rs` and one richer module (e.g. `http.rs` or `shell.rs`) end-to-end plus the central `match` in `mod.rs`, to copy the exact manifest/dispatch-arm/registration idiom.

- [ ] **Step 1: Write failing tests** (same file, `#[cfg(test)]`)

```rust
#[test]
fn onboard_refuses_unconfirmed() {
    let input = serde_json::json!({
        "invite_url": "https://issuer.example.com/onboard#INVAAAA",
        "include_message_text": false,
        "include_tool_payloads": false,
        "confirmed": false,
    });
    let err = validate_onboard_input(&input).unwrap_err();
    assert!(err.to_string().contains("explicit user consent"));
}

#[test]
fn onboard_input_parses_when_confirmed() {
    let input = serde_json::json!({
        "invite_url": "https://issuer.example.com/onboard#INVAAAA",
        "include_message_text": true,
        "include_tool_payloads": false,
        "confirmed": true,
    });
    let parsed = validate_onboard_input(&input).unwrap();
    assert!(parsed.consents.include_message_text);
}

#[test]
fn onboard_success_output_contains_no_key_material() {
    let outcome = ironclaw_reborn_traces::onboarding::OnboardOutcome {
        tenant_id: "tenant-a".into(),
        ingest_url: "https://ingest.example.com".into(),
        issuer_url: "https://issuer.example.com".into(),
        device_key_id: "sha256:abc".into(),
        contributor_label: None,
        community_url: Some("https://tracecommons.ai".into()),
        profile_url: Some("https://tracecommons.ai/profile".into()),
        leaderboard_url: None,
    };
    let out = onboard_success_output(&outcome);
    let s = out.to_string();
    assert!(s.contains("tenant-a"));
    assert!(!s.to_lowercase().contains("private"));
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p ironclaw_host_runtime trace_commons` — compile error.

- [ ] **Step 3: Implement**

```rust
pub const TRACE_COMMONS_ONBOARD_CAPABILITY_ID: &str = "builtin.trace_commons.onboard";
pub const TRACE_COMMONS_STATUS_CAPABILITY_ID: &str = "builtin.trace_commons.status";
```

Manifest descriptions (these are what the model sees — they carry the consent contract):

- onboard: "Enroll this IronClaw in Trace Commons using an operator-issued invite link. ONLY call after the user has explicitly (1) confirmed they want to contribute redacted traces and (2) chosen whether to include redacted message text and redacted tool payloads. Set confirmed=true only when both consents were given in this conversation."
- status: "Report Trace Commons enrollment state for the current user: enrolled or not, tenant, auth mode, consent settings, queue depth."

Parameters schema for onboard (JSON Schema in the `ActionDef`): `invite_url` (string, required), `include_message_text` (boolean, required), `include_tool_payloads` (boolean, required), `confirmed` (boolean, required). Effects: `WriteExternal` + whatever the closest network-using tool declares (copy `http.rs`); `requires_approval`: follow the pattern of other side-effecting tools (set `true` if HTTP-class tools do).

Dispatch for onboard: validate via `validate_onboard_input` (returns typed struct; rejects `confirmed != true` with the message "trace_commons_onboard requires explicit user consent: ask the user to confirm contribution and content inclusion, then call again with confirmed=true"); resolve scope from the request's user identity; call `onboarding::onboard(scope, invite_url, consents)`; map `OnboardError` variants to user-facing messages:
- `InviteRejected(InviteNotValid)` → "This invite link isn't valid — it may have been used already or revoked. Ask the operator for a new invite."
- `IssuerOriginMismatch` → "The server's response didn't match the invite link origin; refusing to continue. The invite may be misconfigured — contact the operator."
- `Network` → "Couldn't reach the onboarding server: {reason}. The invite was not consumed; it's safe to retry."
- `InviteRejected(OnboardRateLimited)` → "The server is rate-limiting onboarding attempts; try again in a few minutes."

Success output (`onboard_success_output`): `{ "enrolled": true, "tenant_id", "ingest_url", "issuer_url", "device_key_id", "consents": {...}, "community_url"?, "profile_url"?, "leaderboard_url"?, "next_steps": "Traces will be redacted locally and queued; submission requires the score threshold. Opt out anytime with 'ironclaw traces opt-out'." }` — include the community/profile/leaderboard URLs only when present on the outcome (already HTTPS-sanitized by the onboarding module) so the agent can point the user at their profile and the leaderboard.

Status dispatch: read the scoped policy via `read_trace_policy_for_scope`, plus queue depth (find the existing queue-status helper the CLI `QueueStatus` subcommand uses in `ironclaw_reborn_cli/src/commands/traces/mod.rs` and call the same `ironclaw_reborn_traces` function). Output: `{ "enrolled": bool, "tenant_id", "auth_mode", "include_message_text", "include_tool_payloads", "queue_depth", "endpoint" }`.

Register both in `builtin_first_party_handlers()` following the existing lines exactly.

- [ ] **Step 4: Run** — `cargo test -p ironclaw_host_runtime trace_commons && cargo check -p ironclaw_reborn_composition` — pass.

- [ ] **Step 5: Commit** — `cargo fmt && git add -A crates/ironclaw_host_runtime && git commit -m "feat(engine): trace_commons onboard and status first-party tools"`

---

### Task 8: Agent conversation prompt

Spec §3.2. Prompt templates live in files, never inline (CLAUDE.md rule).

**Files:**
- Create: `crates/ironclaw_engine/prompts/builtin/trace_commons_onboarding.md`
- Modify: wherever tool-guidance prompts get attached (find how an existing capability's long-form guidance reaches the system prompt — search `include_str!` under `crates/ironclaw_engine/src/` for a `prompts/builtin/` example; if tool guidance is normally carried in the manifest description only, attach this file's content as the onboard tool's extended description/discovery metadata via `ActionDiscoveryMetadata` — match whichever mechanism exists rather than inventing one).

- [ ] **Step 1: Write the prompt file**

```markdown
# Trace Commons onboarding

When the user pastes a Trace Commons invite link (https://…/onboard#CODE, or
mentions an invite code for trace contribution), guide them through enrollment:

1. Explain briefly: Trace Commons collects *redacted* agent traces to improve
   agent quality. Redaction happens locally before anything is uploaded.
   Contribution earns credits. The user can opt out at any time.
2. Ask consent question 1: "Do you want to enroll and contribute redacted
   traces?" Do not proceed without a clear yes.
3. Ask consent question 2: "Should contributions include redacted message
   text, and redacted tool payloads? (Either, both, or neither — metadata-only
   is fine.)"
4. Call trace_commons_onboard with the invite link, the two consent booleans,
   and confirmed=true.
5. Report the result: tenant joined, where data goes, and that they can check
   with trace_commons_status or opt out with `ironclaw traces opt-out`.
   If the result includes profile_url / leaderboard_url / community_url,
   share those links so the user can view their contributor profile and the
   leaderboard in a browser.

Never call trace_commons_onboard with confirmed=true unless steps 2 and 3
happened in this conversation. If the tool reports the invite as not valid,
tell the user to request a fresh invite from the operator — do not retry the
same link more than once.
```

- [ ] **Step 2: Wire it** using the discovered mechanism. Verify with `cargo check -p ironclaw_engine -p ironclaw_host_runtime`.

- [ ] **Step 3: Commit** — `git add -A crates/ironclaw_engine crates/ironclaw_host_runtime && git commit -m "feat(engine): trace commons onboarding conversation guidance prompt"`

---

### Task 9: End-to-end integration test through the tool dispatch path

CLAUDE.md testing rule: the onboarding helper gates side effects (HTTP, policy write) behind the tool wrapper — a unit test on `onboard()` alone is not sufficient; drive the capability dispatch path.

**Files:**
- Create: `crates/ironclaw_host_runtime/tests/trace_commons_onboard_e2e.rs` (or extend the crate's existing integration-test layout — check `crates/ironclaw_host_runtime/tests/` first and follow it)
- Modify: `crates/ironclaw_host_runtime/Cargo.toml` — add `axum = "0.8"` (match the version `ironclaw_reborn_traces` uses) to `[dev-dependencies]`; it is not there today, not even as a dev-dep.

**Base-dir redirection hazard:** `ironclaw_common::paths` honors the `IRONCLAW_BASE_DIR` env var, but it is read ONCE per process into a `LazyLock<PathBuf>` (paths.rs:13). This e2e test works only because integration tests under `tests/` compile to their own binary, so the LazyLock is fresh — set `IRONCLAW_BASE_DIR` to a tempdir at the very top of the test, before ANY call that could touch `ironclaw_base_dir()`. Never rely on `IRONCLAW_BASE_DIR` from inline `#[cfg(test)]` unit tests in this workspace (another test in the same binary may have initialized the LazyLock first) — unit tests must use the dir-parameterized APIs (`onboard_at_dir`, `load_for_tenant` with tempdirs) instead, which is exactly how Tasks 3-6 are written.

- [ ] **Step 1: Write the test**

Spin up the axum mock issuer (reuse/extract the Task 5 test helper into a `#[cfg(test)]`-shared location or duplicate the ~30 lines — duplication is acceptable across crates here rather than a new shared test crate). Then:

1. Set `IRONCLAW_BASE_DIR` to a tempdir (first line of the test; see hazard note above).
2. Build the first-party registry via `builtin_first_party_handlers()`.
3. Dispatch `builtin.trace_commons.onboard` with a `FirstPartyCapabilityRequest` carrying `confirmed: true` and the mock issuer's invite URL.
4. Assert the JSON result reports `enrolled: true`.
5. Dispatch `builtin.trace_commons.status` and assert it reflects enrollment (`auth_mode: "device_key"`, correct tenant).
6. Assert dispatching onboard with `confirmed: false` fails with the consent message and the mock issuer received no second request.

- [ ] **Step 2: Run** — `cargo test -p ironclaw_host_runtime --test trace_commons_onboard_e2e` — pass.

- [ ] **Step 3: Commit** — `cargo fmt && git add -A && git commit -m "test(engine): e2e trace commons onboarding through capability dispatch"`

---

### Task 10: Quality gate + PR

- [ ] **Step 1: Full gate** (CLAUDE.md + memory: full workspace commands, not per-crate)

```bash
cargo fmt --all -- --check
cargo clippy --all --benches --tests --examples --all-features
cargo test
```

Expected: zero warnings, all green. If a `check_no_panics.py` script exists in `scripts/`, run it too.

- [ ] **Step 2: Update docs** — add a short "Agent onboarding" section to `docs/internal/trace-commons.md` describing the invite-link flow, the device-key auth mode, and pointing at the spec.

- [ ] **Step 3: Push and open the PR** (base: `staging`)

```bash
git push -u origin trace-commons-agent-onboarding
gh pr create --base staging --title "feat(traces): agent-driven Trace Commons onboarding via invite link" --body "..."
```

PR body must include, for the designer's onboarding-flow work:
- The **tool contract** (this is the integration surface for any UI): `trace_commons.onboard` input schema + success/error output shapes, `trace_commons.status` output shape — a web onboarding screen can drive exactly these capabilities through the normal dispatch path, and the conversational flow and a future visual flow share one backend.
- The conversation script lives in `crates/ironclaw_engine/prompts/builtin/trace_commons_onboarding.md` — the designer's flow copy can revise that file without code changes.
- Server-side dependency: TraceCommons/trace-commons-server#136-#141 (client merges first; the mock-issuer tests make it independently verifiable; nothing works against the live pilot until #138/#140 deploy).
- Spec link: `docs/superpowers/specs/2026-06-05-trace-commons-agent-onboarding-design.md`.

- [ ] **Step 4: Verify CI passes on the PR.**

---

### Task 11: Credits visibility — console display + agent-queryable balance

**Why:** Once a user opts in, contribution earns credits. They must be able to see in their IronClaw console that credits are accruing and what their current balance is, and the agent must be able to query credit state on demand. This is the payoff side of the opt-in and closes the loop the onboarding flow opens.

**This is mostly a surfacing task — the credit data model already exists** in `crates/ironclaw_reborn_traces/src/contribution.rs`:
- `CreditSummary` (line 782): `submissions_total/submitted/revoked/expired`, `pending_credit`, `final_credit`, `delayed_credit_delta`, `credit_events_total`, `recent_explanations`.
- `TraceCreditReport` (line 829) + `trace_credit_summary(records)` (line 6054) / `trace_credit_report(records)` (line 6069) — compute a summary/report from `LocalTraceSubmissionRecord`s.
- `trace_credit_notice_message(summary)` (line 798) — human-readable one-liner.
- The local submission records live under the scoped trace-contributions dir (`submissions.json`); find the existing loader the CLI `credit` subcommand uses (`crates/ironclaw_reborn_cli/src/commands/traces/mod.rs`) and reuse it — do NOT re-implement record loading.

**Scope:**

1. **Agent-queryable credits tool** — a third first-party capability `builtin.trace_commons.credits` (sibling to `onboard`/`status` from Task 7, same `trace_commons.rs` module + central-dispatch wiring + prompt doc). Read-only (`EffectKind::ReadFilesystem`, `PermissionMode::Allow`). Resolves the scope from `request.scope.user_id.as_str()`, loads the scoped submission records, runs `trace_credit_summary`/`trace_credit_report`, and returns a structured `Value`: `{ enrolled, pending_credit, final_credit, delayed_credit_delta, submissions_submitted, submissions_total, credit_events_total, recent_explanations, as_of }`. If not enrolled / no records, return a clean `{ enrolled: false }`-style value, not an error. Pure formatter `format_credits(summary, report) -> Value` is unit-tested with hand-built summaries (no disk). Extend the Task 8 onboarding prompt (or add a `trace-commons-credits.md` prompt doc) so the agent knows to call this when the user asks "how are my trace credits doing?".

2. **Console display** — the web BACKEND already exists: `GET /api/traces/credit` (`src/channels/web/handlers/traces.rs:349`, `traces_credit_handler`, route in `platform/router.rs`) already returns `TraceCreditResponse { summary, report, records }` via `TraceClientHost::read_local_records_for_scope` + `trace_credit_summary`/`trace_credit_report`. It follows the established trace-handler pattern (TraceClientHost, like all ten `/api/traces/*` handlers — do NOT rework them to ToolDispatcher or add a duplicate dispatch endpoint). The remaining gap is purely **frontend**: there is currently NO trace settings panel in the gateway JS. Building that panel overlaps the designer's in-flight onboarding/console flow, so this sub-item is **coordinate-with-designer**: either (a) a minimal self-contained credits card fetching `/api/traces/credit` showing pending + final balance, submissions count, and the `recent_explanations` ledger (framed pending vs. final vs. delayed so it doesn't over-promise — reuse `trace_credit_notice_message`'s framing), or (b) defer the visual surface to the designer and ship only the agent tool plus the already-present endpoint. Confirm direction before building throwaway UI.

3. **Tests:** unit-test `format_credits` (enrolled-with-credits, enrolled-no-records, not-enrolled); drive the credits capability through `ToolDispatcher::dispatch()` at the integration tier (per the test-through-the-caller rule) asserting the dispatched output shape; if a web endpoint is added, a handler test asserting it routes through dispatch.

**Quality gates:** same as every task — `cargo test`, `cargo clippy -p ironclaw_host_runtime --all-features --tests --examples -- -D warnings`, the web crate's clippy if a handler is added, `cargo fmt`. Commit: `feat(traces): credits visibility — agent-queryable balance and console display`.

**Coordination note:** the *authoritative* credit ledger is server-side (TraceCommons/trace-commons-server, the `near_credit`/credit modules); the local `CreditSummary` is the client's view derived from submission records + server status sync. The console/agent surface shows the local view and should label it as such ("as last synced"). A future server endpoint for the canonical balance can swap in behind the same tool contract — keep the tool output shape stable so the UI doesn't churn.
