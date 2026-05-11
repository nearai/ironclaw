//! Admin secrets provisioning handlers.
//!
//! Allows an admin (typically an application backend) to create, list, and
//! delete secrets on behalf of individual users so their BastionClaw agent can
//! call back to external services with per-user credentials.

use std::sync::{Arc, LazyLock};

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use base64::Engine as _;
use regex::Regex;

use crate::channels::web::auth::AdminUser;
use crate::channels::web::server::GatewayState;
use crate::secrets::CreateSecretParams;

// ── Byte-length constants mirrored from client/t3n-sdk/src/client/delegation.ts ──
const ETH_SIG_LEN: usize = 65;
const AGENT_PUBKEY_LEN: usize = 33;

static ORG_DID_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^did:t3n:[0-9a-f]{40}$").expect("static regex"));

/// Reasons a `t3n_delegation_token` value can fail shape validation.
///
/// Each variant corresponds to exactly one rejection path; the handler converts
/// each to a structured `{ code, field, reason }` JSON body.
#[derive(Debug)]
enum DelegationTokenValidationError {
    InvalidJson { reason: String },
    MissingField { field: &'static str },
    WrongType { field: &'static str, expected: &'static str },
    InvalidB64u { field: &'static str, reason: String },
    WrongByteLength { field: &'static str, expected: usize, actual: usize },
    InnerJsonInvalid { reason: String },
    MissingInnerField { field: &'static str },
    WrongInnerType { field: &'static str, expected: &'static str },
    InvalidOrgDidShape { value: String },
}

impl DelegationTokenValidationError {
    fn field(&self) -> &str {
        match self {
            Self::InvalidJson { .. } => "<root>",
            Self::MissingField { field } => field,
            Self::WrongType { field, .. } => field,
            Self::InvalidB64u { field, .. } => field,
            Self::WrongByteLength { field, .. } => field,
            Self::InnerJsonInvalid { .. } => "credential_jcs",
            Self::MissingInnerField { field } => field,
            Self::WrongInnerType { field, .. } => field,
            Self::InvalidOrgDidShape { .. } => "org_did",
        }
    }

    fn reason(&self) -> String {
        match self {
            Self::InvalidJson { reason } => reason.clone(),
            Self::MissingField { field } => format!("required field '{field}' is missing"),
            Self::WrongType { field, expected } => {
                format!("field '{field}' must be a {expected}")
            }
            Self::InvalidB64u { field, reason } => {
                format!("field '{field}' is not valid base64url: {reason}")
            }
            Self::WrongByteLength { field, expected, actual } => {
                format!(
                    "field '{field}' must be {expected} bytes after base64url decode, got {actual}"
                )
            }
            Self::InnerJsonInvalid { reason } => {
                format!("credential_jcs does not decode to valid JSON: {reason}")
            }
            Self::MissingInnerField { field } => {
                format!("credential_jcs is missing required inner field '{field}'")
            }
            Self::WrongInnerType { field, expected } => {
                format!("credential_jcs inner field '{field}' must be a {expected}")
            }
            Self::InvalidOrgDidShape { value } => {
                format!(
                    "org_did '{value}' must match did:t3n:<40 lowercase hex> \
                     (e.g. did:t3n:a1b2c3…)"
                )
            }
        }
    }

    fn to_response(&self) -> (StatusCode, Json<serde_json::Value>) {
        let body = serde_json::json!({
            "code": "invalid_secret_shape",
            "field": self.field(),
            "reason": self.reason(),
        });
        (StatusCode::BAD_REQUEST, Json(body))
    }
}

/// Validate the shape of a `t3n_delegation_token` secret value before persisting.
///
/// Pure function: takes the raw value string, returns `Ok(())` on success.
/// The caller is responsible for rejecting the PUT and returning the structured
/// 400 body from `DelegationTokenValidationError::to_response`.
fn validate_delegation_token(value: &str) -> Result<(), DelegationTokenValidationError> {
    let b64u = base64::engine::general_purpose::URL_SAFE_NO_PAD;

    // ── 1. Outer JSON parse ────────────────────────────────────────────────────
    let token: serde_json::Value =
        serde_json::from_str(value).map_err(|e| DelegationTokenValidationError::InvalidJson {
            reason: e.to_string(),
        })?;

    // ── 2. Required top-level string fields ───────────────────────────────────
    let get_str = |field: &'static str| -> Result<&str, DelegationTokenValidationError> {
        match token.get(field) {
            None => Err(DelegationTokenValidationError::MissingField { field }),
            Some(v) => v
                .as_str()
                .ok_or(DelegationTokenValidationError::WrongType { field, expected: "string" }),
        }
    };

    let credential_jcs = get_str("credential_jcs")?.to_string();
    let user_sig = get_str("user_sig")?.to_string();
    let agent_pubkey = get_str("agent_pubkey")?.to_string();

    // ── 3. user_sig: b64url decode, assert 65 bytes ───────────────────────────
    let sig_bytes = b64u
        .decode(user_sig.trim_end_matches('='))
        .map_err(|e| DelegationTokenValidationError::InvalidB64u {
            field: "user_sig",
            reason: e.to_string(),
        })?;
    if sig_bytes.len() != ETH_SIG_LEN {
        return Err(DelegationTokenValidationError::WrongByteLength {
            field: "user_sig",
            expected: ETH_SIG_LEN,
            actual: sig_bytes.len(),
        });
    }

    // ── 4. agent_pubkey: hex (optional 0x prefix) or b64url, assert 33 bytes ──
    let pubkey_bytes = {
        let s = agent_pubkey.trim_start_matches("0x");
        // Attempt hex first: must be exactly 66 hex chars (33 bytes × 2).
        if s.len() == 66 && s.chars().all(|c| c.is_ascii_hexdigit()) {
            hex::decode(s).map_err(|e| DelegationTokenValidationError::InvalidB64u {
                field: "agent_pubkey",
                reason: e.to_string(),
            })?
        } else {
            b64u
                .decode(agent_pubkey.trim_end_matches('='))
                .map_err(|e| DelegationTokenValidationError::InvalidB64u {
                    field: "agent_pubkey",
                    reason: e.to_string(),
                })?
        }
    };
    if pubkey_bytes.len() != AGENT_PUBKEY_LEN {
        return Err(DelegationTokenValidationError::WrongByteLength {
            field: "agent_pubkey",
            expected: AGENT_PUBKEY_LEN,
            actual: pubkey_bytes.len(),
        });
    }

    // ── 5. credential_jcs: b64url decode, parse inner JSON ────────────────────
    let jcs_bytes =
        b64u.decode(credential_jcs.trim_end_matches('=')).map_err(|e| {
            DelegationTokenValidationError::InvalidB64u {
                field: "credential_jcs",
                reason: e.to_string(),
            }
        })?;
    let inner: serde_json::Value =
        serde_json::from_slice(&jcs_bytes).map_err(|e| {
            DelegationTokenValidationError::InnerJsonInvalid {
                reason: e.to_string(),
            }
        })?;

    // ── 6. Required inner string fields ───────────────────────────────────────
    let get_inner_str =
        |field: &'static str| -> Result<&str, DelegationTokenValidationError> {
            match inner.get(field) {
                None => Err(DelegationTokenValidationError::MissingInnerField { field }),
                Some(v) => v.as_str().ok_or(
                    DelegationTokenValidationError::WrongInnerType { field, expected: "string" },
                ),
            }
        };

    let org_did = get_inner_str("org_did")?.to_string();
    get_inner_str("vc_id")?;
    get_inner_str("user_did")?;

    // `not_before_secs` / `not_after_secs` are numbers on the wire; accepted as
    // either number or string per the spec note.  We only check presence here.
    if inner.get("not_before_secs").is_none() {
        return Err(DelegationTokenValidationError::MissingInnerField {
            field: "not_before_secs",
        });
    }
    if inner.get("not_after_secs").is_none() {
        return Err(DelegationTokenValidationError::MissingInnerField {
            field: "not_after_secs",
        });
    }

    // ── 7. org_did must be a fully-qualified did:t3n:<40 lowercase hex> ───────
    if !ORG_DID_RE.is_match(&org_did) {
        return Err(DelegationTokenValidationError::InvalidOrgDidShape { value: org_did });
    }

    Ok(())
}

/// PUT /api/admin/users/{user_id}/secrets/{name} — create or update a secret.
///
/// Upserts: if a secret with the same (user_id, name) already exists it is
/// overwritten. The plaintext value is encrypted at rest (AES-256-GCM) and
/// never returned by any endpoint.
///
/// When `name` is `t3n_delegation_token`, the value is validated for shape
/// before persisting. A malformed token is rejected immediately (HTTP 400)
/// so the operator learns about the problem at upload time rather than ~30 s
/// later when `runPayroll` fails.
pub async fn secrets_put_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_admin): AdminUser,
    Path((user_id, name)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let name = name.to_lowercase();

    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({"error": "Database not available"})),
    ))?;
    store
        .get_user(&user_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "User not found"})),
        ))?;

    let secrets = state.secrets_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({"error": "Secrets store not available"})),
    ))?;

    let value = body
        .get("value")
        .and_then(|v| v.as_str())
        .ok_or((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing required field 'value'"})),
        ))?
        .to_string();

    // Per-secret shape validation before persisting.
    if name == crate::tools::mcp::config::T3N_DELEGATION_TOKEN_SECRET {
        if let Err(e) = validate_delegation_token(&value) {
            let (status, body) = e.to_response();
            return Err((status, body));
        }
    }

    let provider = body
        .get("provider")
        .and_then(|v| v.as_str())
        .map(String::from);

    let expires_in_days = body.get("expires_in_days").and_then(|v| v.as_u64());
    if let Some(days) = expires_in_days
        && days > 36500
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "expires_in_days must be at most 36500"})),
        ));
    }
    let expires_at =
        expires_in_days.map(|days| chrono::Utc::now() + chrono::Duration::days(days as i64));

    let mut params = CreateSecretParams::new(name.clone(), value);
    if let Some(p) = provider {
        params = params.with_provider(p);
    }
    if let Some(exp) = expires_at {
        params = params.with_expiry(exp);
    }

    let already_exists = secrets
        .exists(&user_id, &name)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
        })?;

    secrets
        .create(&user_id, params)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
        })?;

    Ok(Json(serde_json::json!({
        "user_id": user_id,
        "name": name,
        "status": if already_exists { "updated" } else { "created" },
    })))
}

/// GET /api/admin/users/{user_id}/secrets — list a user's secrets (names only).
///
/// Never returns secret values or hashes.
pub async fn secrets_list_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_admin): AdminUser,
    Path(user_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Verify the target user exists (consistent with PUT/DELETE).
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    if store
        .get_user(&user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .is_none()
    {
        return Err((StatusCode::NOT_FOUND, "User not found".to_string()));
    }

    let secrets = state.secrets_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Secrets store not available".to_string(),
    ))?;

    let refs = secrets
        .list(&user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let secrets_json: Vec<serde_json::Value> = refs
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "name": r.name,
                "provider": r.provider,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "user_id": user_id,
        "secrets": secrets_json,
    })))
}

/// DELETE /api/admin/users/{user_id}/secrets/{name} — delete a user's secret.
pub async fn secrets_delete_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_admin): AdminUser,
    Path((user_id, name)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let name = name.to_lowercase();

    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    store
        .get_user(&user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    let secrets = state.secrets_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Secrets store not available".to_string(),
    ))?;

    let deleted = secrets
        .delete(&user_id, &name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !deleted {
        return Err((StatusCode::NOT_FOUND, "Secret not found".to_string()));
    }

    Ok(Json(serde_json::json!({
        "user_id": user_id,
        "name": name,
        "deleted": true,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Test helpers ──────────────────────────────────────────────────────────

    fn b64u_encode(bytes: &[u8]) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    /// Build a valid inner JCS JSON and return it as a base64url string.
    fn valid_credential_jcs_b64u(org_did: &str) -> String {
        let inner = serde_json::json!({
            "vc_id": "AAAAAAAAAAAAAAAAAAAAAA",
            "user_did": "did:t3n:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "org_did": org_did,
            "not_before_secs": 1700000000u64,
            "not_after_secs": 1800000000u64,
        });
        b64u_encode(inner.to_string().as_bytes())
    }

    /// Build a complete valid delegation token JSON string.
    fn valid_token() -> String {
        let sig = b64u_encode(&[0xABu8; ETH_SIG_LEN]);
        let pubkey = b64u_encode(&[0xCDu8; AGENT_PUBKEY_LEN]);
        let cjcs = valid_credential_jcs_b64u("did:t3n:a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2");
        serde_json::json!({
            "credential_jcs": cjcs,
            "user_sig": sig,
            "agent_pubkey": pubkey,
        })
        .to_string()
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[test]
    fn valid_token_passes() {
        assert!(validate_delegation_token(&valid_token()).is_ok());
    }

    #[test]
    fn rejects_malformed_outer_json() {
        let err = validate_delegation_token("not-json{{{").unwrap_err();
        assert!(
            matches!(err, DelegationTokenValidationError::InvalidJson { .. }),
            "expected InvalidJson, got: {err:?}"
        );
        assert_eq!(err.field(), "<root>");
    }

    #[test]
    fn rejects_missing_credential_jcs() {
        let mut v: serde_json::Value = serde_json::from_str(&valid_token()).unwrap();
        v.as_object_mut().unwrap().remove("credential_jcs");
        let err = validate_delegation_token(&v.to_string()).unwrap_err();
        assert!(matches!(
            err,
            DelegationTokenValidationError::MissingField { field: "credential_jcs" }
        ));
    }

    #[test]
    fn rejects_missing_user_sig() {
        let mut v: serde_json::Value = serde_json::from_str(&valid_token()).unwrap();
        v.as_object_mut().unwrap().remove("user_sig");
        let err = validate_delegation_token(&v.to_string()).unwrap_err();
        assert!(matches!(
            err,
            DelegationTokenValidationError::MissingField { field: "user_sig" }
        ));
    }

    #[test]
    fn rejects_missing_agent_pubkey() {
        let mut v: serde_json::Value = serde_json::from_str(&valid_token()).unwrap();
        v.as_object_mut().unwrap().remove("agent_pubkey");
        let err = validate_delegation_token(&v.to_string()).unwrap_err();
        assert!(matches!(
            err,
            DelegationTokenValidationError::MissingField { field: "agent_pubkey" }
        ));
    }

    #[test]
    fn rejects_wrong_user_sig_length() {
        // 39 bytes — short of a valid 65-byte ETH signature.
        let short_sig = b64u_encode(&[0xABu8; 39]);
        let mut v: serde_json::Value = serde_json::from_str(&valid_token()).unwrap();
        v["user_sig"] = serde_json::Value::String(short_sig);
        let err = validate_delegation_token(&v.to_string()).unwrap_err();
        assert!(
            matches!(
                err,
                DelegationTokenValidationError::WrongByteLength {
                    field: "user_sig",
                    expected: ETH_SIG_LEN,
                    actual: 39
                }
            ),
            "expected WrongByteLength for user_sig, got: {err:?}"
        );
        assert!(err.reason().contains("39"));
    }

    #[test]
    fn rejects_wrong_agent_pubkey_length() {
        let short_key = b64u_encode(&[0xCDu8; 32]);
        let mut v: serde_json::Value = serde_json::from_str(&valid_token()).unwrap();
        v["agent_pubkey"] = serde_json::Value::String(short_key);
        let err = validate_delegation_token(&v.to_string()).unwrap_err();
        assert!(matches!(
            err,
            DelegationTokenValidationError::WrongByteLength {
                field: "agent_pubkey",
                expected: AGENT_PUBKEY_LEN,
                actual: 32
            }
        ));
    }

    #[test]
    fn accepts_agent_pubkey_as_hex() {
        let hex_pubkey = hex::encode(&[0xCDu8; AGENT_PUBKEY_LEN]);
        let mut v: serde_json::Value = serde_json::from_str(&valid_token()).unwrap();
        v["agent_pubkey"] = serde_json::Value::String(hex_pubkey);
        assert!(validate_delegation_token(&v.to_string()).is_ok());
    }

    #[test]
    fn accepts_agent_pubkey_as_0x_hex() {
        let hex_pubkey = format!("0x{}", hex::encode(&[0xCDu8; AGENT_PUBKEY_LEN]));
        let mut v: serde_json::Value = serde_json::from_str(&valid_token()).unwrap();
        v["agent_pubkey"] = serde_json::Value::String(hex_pubkey);
        assert!(validate_delegation_token(&v.to_string()).is_ok());
    }

    #[test]
    fn rejects_malformed_inner_jcs_json() {
        let bad_inner = b64u_encode(b"not-json{{{");
        let mut v: serde_json::Value = serde_json::from_str(&valid_token()).unwrap();
        v["credential_jcs"] = serde_json::Value::String(bad_inner);
        let err = validate_delegation_token(&v.to_string()).unwrap_err();
        assert!(
            matches!(err, DelegationTokenValidationError::InnerJsonInvalid { .. }),
            "expected InnerJsonInvalid, got: {err:?}"
        );
    }

    #[test]
    fn rejects_bare_hex_org_did() {
        // Operator copies raw 40-hex from admin-organisation.sh without did:t3n: prefix.
        let jcs = valid_credential_jcs_b64u("a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2");
        let mut v: serde_json::Value = serde_json::from_str(&valid_token()).unwrap();
        v["credential_jcs"] = serde_json::Value::String(jcs);
        let err = validate_delegation_token(&v.to_string()).unwrap_err();
        assert!(
            matches!(err, DelegationTokenValidationError::InvalidOrgDidShape { .. }),
            "expected InvalidOrgDidShape, got: {err:?}"
        );
        assert!(err.reason().contains("did:t3n:"));
    }

    #[test]
    fn rejects_uppercase_hex_in_org_did() {
        let jcs = valid_credential_jcs_b64u("did:t3n:A1B2C3D4E5F6A1B2C3D4E5F6A1B2C3D4E5F6A1B2");
        let mut v: serde_json::Value = serde_json::from_str(&valid_token()).unwrap();
        v["credential_jcs"] = serde_json::Value::String(jcs);
        let err = validate_delegation_token(&v.to_string()).unwrap_err();
        assert!(matches!(
            err,
            DelegationTokenValidationError::InvalidOrgDidShape { .. }
        ));
    }
}
