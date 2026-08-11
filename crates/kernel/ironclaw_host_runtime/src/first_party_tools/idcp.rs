//! Host-mediated IdentyClaw helper (`builtin.idcp`).
//!
//! Talks to the loopback IdentyClaw sidecar (`IDENTYCLAW_HELPER_BASE`, default
//! `http://127.0.0.1:3921`) — the same surface as the `idcp` CLI. Passport keys
//! and JWTs stay on the host; this capability never returns them to the model.
//!
//! Declares only `DispatchCapability` (not `Network` / `SpawnProcess`) so it
//! remains visible under processless profiles such as
//! `hosted-single-tenant-volume`. Destinations are host-configured loopback
//! only; the model cannot supply arbitrary URLs.

use std::sync::Mutex;
use std::time::Duration;

use ironclaw_extension_registry::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{
    capability::{EffectKind, PermissionMode},
    dispatch::RuntimeDispatchErrorKind,
    resource::{ResourceCeiling, ResourceEstimate, ResourceProfile},
};
use serde_json::{Map, Value, json};
use url::Url;

use crate::FirstPartyCapabilityError;

use super::{
    FIRST_PARTY_DEFAULT_OUTPUT_BYTES, FIRST_PARTY_MAX_OUTPUT_BYTES,
    first_party_capability_manifest, input_error,
};

pub const IDCP_CAPABILITY_ID: &str = "builtin.idcp";

const DEFAULT_HELPER_BASE: &str = "http://127.0.0.1:3921";
const HELPER_ENV: &str = "IDENTYCLAW_HELPER_BASE";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_RESPONSE_BYTES: usize = 256 * 1024;

/// Exact key names redacted from model-visible helper JSON. Prefer exact matches
/// over substring checks so metadata like `jwt_length` / `tokenId` stay intact.
const SENSITIVE_KEYS: &[&str] = &[
    "jwt",
    "authorization",
    "private_key",
    "privatekey",
    "secret",
    "password",
    "access_token",
    "refresh_token",
    "bearer",
    "credential",
    "credentials",
];

#[doc(hidden)]
static TEST_HELPER_BASE_OVERRIDE: Mutex<Option<String>> = Mutex::new(None);

/// Test-only override for the loopback helper base (avoids process-wide env races).
#[doc(hidden)]
pub fn set_test_helper_base_override(base: Option<String>) {
    if let Ok(mut guard) = TEST_HELPER_BASE_OVERRIDE.lock() {
        *guard = base;
    }
}
pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        IDCP_CAPABILITY_ID,
        "IdentyClaw Passport helpers via the host idcp helper (ensure_session, me, \
         request, create_hola, verify_hola, agents, info). Prefer this over inventing \
         signatures or pasting JWTs. Never returns private keys or full JWTs. \
         Federated login: pass base=<peer https URL> on ensure_session only — ok+federated \
         means login succeeded; do not call me against the peer (home-only). Omit base for \
         home https://api.identyclaw.com. Do not pass the loopback helper URL as base.",
        vec![EffectKind::DispatchCapability],
        PermissionMode::Allow,
        Some(ResourceProfile {
            default_estimate: ResourceEstimate::default()
                .set_wall_clock_ms(15_000)
                .set_output_bytes(FIRST_PARTY_DEFAULT_OUTPUT_BYTES),
            hard_ceiling: Some(ResourceCeiling {
                max_usd: None,
                max_input_tokens: None,
                max_output_tokens: None,
                max_wall_clock_ms: Some(30_000),
                max_output_bytes: Some(FIRST_PARTY_MAX_OUTPUT_BYTES),
                sandbox: None,
            }),
        }),
    )
}

pub(super) async fn dispatch(input: &Value) -> Result<Value, FirstPartyCapabilityError> {
    let op = input
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(input_error)?;
    let base = optional_string(input, "base")?
        .or(optional_string(input, "apiEndpoint")?)
        .or(optional_string(input, "api_endpoint")?);
    let plan = match op {
        "ensure_session" | "ensure-session" => {
            HelperCall::post("/v1/ensure_session", ensure_body_with_base(base.as_deref()))
        }
        "list_sessions" | "list-sessions" => HelperCall::get("/v1/sessions"),
        "me" => {
            if let Some(api) = base.as_deref() {
                let enc = urlencoding_minimal(api);
                HelperCall::get(&format!("/v1/me?apiEndpoint={enc}"))
            } else {
                HelperCall::get("/v1/me")
            }
        }
        "info" => HelperCall::get("/v1/info"),
        "agents" => {
            let limit = input
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(20)
                .clamp(1, 100);
            HelperCall::get(&format!("/v1/agents?limit={limit}"))
        }
        "request" => {
            let method = required_string(input, "method")?;
            let path = required_string(input, "path")?;
            if !path.starts_with('/') {
                return Err(input_error());
            }
            let mut payload = Map::new();
            payload.insert("method".into(), Value::String(method));
            payload.insert("path".into(), Value::String(path));
            if let Some(api) = base {
                payload.insert("apiEndpoint".into(), Value::String(api));
            }
            if let Some(body) = input.get("body")
                && !body.is_null()
            {
                payload.insert("body".into(), body.clone());
            }
            HelperCall::post("/v1/request", Value::Object(payload))
        }
        "create_hola" | "create-hola" => {
            let recipient = input
                .get("recipient")
                .and_then(Value::as_str)
                .unwrap_or("MUNDO");
            let mut payload = Map::new();
            payload.insert("recipient".into(), Value::String(recipient.to_string()));
            if let Some(api) = base {
                payload.insert("apiEndpoint".into(), Value::String(api));
            }
            HelperCall::post("/v1/create_hola", Value::Object(payload))
        }
        "verify_hola" | "verify-hola" => {
            let hola = required_string(input, "hola")?;
            let mut payload = Map::new();
            payload.insert("hola".into(), Value::String(hola));
            if let Some(expected) = optional_string(input, "expected")? {
                payload.insert("expectedRecipient".into(), Value::String(expected));
            }
            if let Some(api) = base {
                payload.insert("apiEndpoint".into(), Value::String(api));
            }
            HelperCall::post("/v1/verify_hola", Value::Object(payload))
        }
        "enroll" => {
            return Ok(json!({
                "ok": false,
                "error": "enroll_host_only",
                "hint": "Run on the host: idcp enroll (see deploy/identyclaw/README.md)"
            }));
        }
        _ => return Err(input_error()),
    };

    match call_helper(&plan).await {
        Ok(body) => Ok(redact_sensitive(body)),
        Err(HelperError::Unreachable { detail }) => Ok(json!({
            "ok": false,
            "error": "identyclaw_helper_unreachable",
            "detail": detail,
            "hint": "Operator: start the host helper (`cd deploy/identyclaw && npm start`) and set IDENTYCLAW_HELPER_BASE=http://127.0.0.1:3921"
        })),
        Err(HelperError::InvalidBase { detail }) => {
            Err(FirstPartyCapabilityError::with_safe_summary(
                RuntimeDispatchErrorKind::OperationFailed,
                format!("invalid IDENTYCLAW_HELPER_BASE: {detail}"),
            ))
        }
        Err(HelperError::BadResponse { detail }) => Ok(json!({
            "ok": false,
            "error": "identyclaw_helper_bad_response",
            "detail": detail
        })),
    }
}

struct HelperCall {
    method: &'static str,
    path: String,
    body: Option<Value>,
}

impl HelperCall {
    fn get(path: &str) -> Self {
        Self {
            method: "GET",
            path: path.to_string(),
            body: None,
        }
    }

    fn post(path: &str, body: Value) -> Self {
        Self {
            method: "POST",
            path: path.to_string(),
            body: Some(body),
        }
    }
}

#[derive(Debug)]
enum HelperError {
    InvalidBase { detail: String },
    Unreachable { detail: String },
    BadResponse { detail: String },
}

async fn call_helper(plan: &HelperCall) -> Result<Value, HelperError> {
    let base = resolve_helper_base()?;
    let url = format!(
        "{base}{path}",
        base = base.trim_end_matches('/'),
        path = plan.path
    );
    let client = reqwest::Client::builder()
        .timeout(DEFAULT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|err| HelperError::Unreachable {
            detail: err.to_string(),
        })?;

    let mut request = match plan.method {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        _ => {
            return Err(HelperError::BadResponse {
                detail: format!("unsupported helper method {}", plan.method),
            });
        }
    };
    request = request.header("accept", "application/json");
    if let Some(body) = &plan.body {
        request = request
            .header("content-type", "application/json")
            .json(body);
    }

    let response = request
        .send()
        .await
        .map_err(|err| HelperError::Unreachable {
            detail: truncate_detail(&err.to_string()),
        })?;
    let status = response.status().as_u16();
    let bytes = response
        .bytes()
        .await
        .map_err(|err| HelperError::Unreachable {
            detail: truncate_detail(&err.to_string()),
        })?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(HelperError::BadResponse {
            detail: format!("response exceeded {MAX_RESPONSE_BYTES} bytes"),
        });
    }
    let parsed: Value = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes).map_err(|err| HelperError::BadResponse {
            detail: truncate_detail(&err.to_string()),
        })?
    };
    if !(200..300).contains(&status) {
        // Helper returns JSON error bodies on 4xx/5xx — surface them redacted.
        return Ok(match parsed {
            Value::Object(mut map) => {
                map.entry("ok".to_string()).or_insert(Value::Bool(false));
                map.insert("http_status".into(), json!(status));
                Value::Object(map)
            }
            other => json!({
                "ok": false,
                "http_status": status,
                "body": other
            }),
        });
    }
    Ok(parsed)
}

fn resolve_helper_base() -> Result<String, HelperError> {
    if let Ok(guard) = TEST_HELPER_BASE_OVERRIDE.lock()
        && let Some(base) = guard.as_ref()
    {
        return validate_loopback_helper_base(base);
    }
    let raw = std::env::var(HELPER_ENV).unwrap_or_else(|_| DEFAULT_HELPER_BASE.to_string());
    validate_loopback_helper_base(&raw)
}

fn validate_loopback_helper_base(raw: &str) -> Result<String, HelperError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(HelperError::InvalidBase {
            detail: "empty".into(),
        });
    }
    let url = Url::parse(trimmed).map_err(|err| HelperError::InvalidBase {
        detail: err.to_string(),
    })?;
    if url.scheme() != "http" {
        return Err(HelperError::InvalidBase {
            detail: "only http:// loopback helpers are allowed".into(),
        });
    }
    if url.username() != "" || url.password().is_some() {
        return Err(HelperError::InvalidBase {
            detail: "credentials in helper base are not allowed".into(),
        });
    }
    let host = url.host_str().unwrap_or("");
    if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return Err(HelperError::InvalidBase {
            detail: format!("host must be loopback, got {host}"),
        });
    }
    if url.path() != "/" && !url.path().is_empty() {
        return Err(HelperError::InvalidBase {
            detail: "helper base must not include a path".into(),
        });
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(HelperError::InvalidBase {
            detail: "helper base must not include query or fragment".into(),
        });
    }
    Ok(trimmed.trim_end_matches('/').to_string())
}

fn ensure_body_with_base(base: Option<&str>) -> Value {
    match base {
        Some(api) => json!({ "apiEndpoint": api }),
        None => json!({}),
    }
}

fn required_string(input: &Value, key: &str) -> Result<String, FirstPartyCapabilityError> {
    input
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(input_error)
}

fn optional_string(input: &Value, key: &str) -> Result<Option<String>, FirstPartyCapabilityError> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.is_empty() || value == "null" => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(input_error()),
    }
}

fn urlencoding_minimal(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(ch),
            _ => {
                for byte in ch.to_string().as_bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}

fn redact_sensitive(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, child) in map {
                if is_sensitive_key(&key) {
                    out.insert(key, Value::String("[redacted]".into()));
                } else {
                    out.insert(key, redact_sensitive(child));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(redact_sensitive).collect()),
        Value::String(s) if looks_like_jwt(&s) => Value::String("[redacted-jwt]".into()),
        other => other,
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SENSITIVE_KEYS.iter().any(|candidate| lower == *candidate)
        || lower.ends_with("_jwt")
        || lower.ends_with("_secret")
        || lower.ends_with("_password")
        || lower.ends_with("_private_key")
}

fn looks_like_jwt(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.len() >= 8
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
}

fn truncate_detail(detail: &str) -> String {
    const MAX: usize = 240;
    if detail.len() <= MAX {
        detail.to_string()
    } else {
        format!("{}…", &detail[..MAX])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_default_loopback_base() {
        assert_eq!(
            validate_loopback_helper_base("http://127.0.0.1:3921").unwrap(),
            "http://127.0.0.1:3921"
        );
    }

    #[test]
    fn rejects_non_loopback_helper_base() {
        assert!(validate_loopback_helper_base("http://api.identyclaw.com").is_err());
        assert!(validate_loopback_helper_base("https://127.0.0.1:3921").is_err());
        assert!(validate_loopback_helper_base("http://127.0.0.1:3921/v1").is_err());
    }

    #[test]
    fn redacts_jwt_shaped_strings_and_sensitive_keys() {
        let input = json!({
            "ok": true,
            "jwt": "aaaa.bbbb.cccc",
            "jwt_length": 1920,
            "nested": { "access_token": "secret", "tokenId": "abc" },
            "body": "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.signaturepart"
        });
        let redacted = redact_sensitive(input);
        assert_eq!(redacted["jwt"], json!("[redacted]"));
        assert_eq!(redacted["jwt_length"], json!(1920));
        assert_eq!(redacted["nested"]["access_token"], json!("[redacted]"));
        assert_eq!(redacted["nested"]["tokenId"], json!("abc"));
        assert_eq!(redacted["body"], json!("[redacted-jwt]"));
    }

    #[test]
    fn ensure_body_accepts_peer_api_endpoint() {
        assert_eq!(
            ensure_body_with_base(Some("https://peer.example.com")),
            json!({ "apiEndpoint": "https://peer.example.com" })
        );
        assert_eq!(ensure_body_with_base(None), json!({}));
    }

    #[tokio::test]
    async fn accepts_hyphenated_op_and_api_endpoint_alias() {
        // Without a helper, dispatch still accepts the aliases and returns a
        // structured unreachable result (not InputEncode).
        let out = dispatch(&json!({
            "op": "ensure-session",
            "apiEndpoint": "https://peer.example.com"
        }))
        .await
        .expect("aliases must not InputEncode");
        assert_eq!(out["ok"], json!(false));
        assert_eq!(out["error"], json!("identyclaw_helper_unreachable"));
    }
}
