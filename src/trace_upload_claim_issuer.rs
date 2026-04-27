use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::errors::ErrorKind as JwtErrorKind;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::trace_contribution::{ConsentScope, TraceAllowedUse};

pub const TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION: &str =
    "ironclaw.trace_upload_claim_request.v1";
const DEFAULT_BIND: &str = "127.0.0.1:3917";
const DEFAULT_MAX_TTL_SECONDS: i64 = 300;

#[derive(Debug, Clone)]
pub struct TraceUploadClaimIssuerConfig {
    pub bind: SocketAddr,
    pub signing_private_key_pem: String,
    pub signing_public_key_pem: String,
    pub signing_kid: String,
    pub issuer: String,
    pub audience: String,
    pub max_ttl_seconds: i64,
    pub workload_public_key_pem: String,
    pub workload_issuer: Option<String>,
    pub workload_audience: Option<String>,
}

impl TraceUploadClaimIssuerConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind = optional_env("TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_BIND")?
            .unwrap_or_else(|| DEFAULT_BIND.to_string())
            .parse()
            .context("invalid TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_BIND")?;
        let signing_private_key_pem = required_pem_or_file(
            "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_PRIVATE_KEY_PEM",
            "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_PRIVATE_KEY_FILE",
        )?;
        let signing_public_key_pem = required_pem_or_file(
            "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_PUBLIC_KEY_PEM",
            "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_PUBLIC_KEY_FILE",
        )?;
        let workload_public_key_pem = required_pem_or_file(
            "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_WORKLOAD_PUBLIC_KEY_PEM",
            "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_WORKLOAD_PUBLIC_KEY_FILE",
        )?;
        let max_ttl_seconds = optional_env("TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_MAX_TTL_SECONDS")?
            .map(|value| {
                value
                    .parse::<i64>()
                    .context("invalid TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_MAX_TTL_SECONDS")
            })
            .transpose()?
            .unwrap_or(DEFAULT_MAX_TTL_SECONDS);

        Ok(Self {
            bind,
            signing_private_key_pem,
            signing_public_key_pem,
            signing_kid: required_env("TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_KID")?,
            issuer: required_env("TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_ISSUER")?,
            audience: required_env("TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_AUDIENCE")?,
            max_ttl_seconds,
            workload_public_key_pem,
            workload_issuer: optional_env("TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_WORKLOAD_ISSUER")?,
            workload_audience: optional_env("TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_WORKLOAD_AUDIENCE")?,
        })
    }

    fn build_state(&self) -> anyhow::Result<Arc<TraceUploadClaimIssuerState>> {
        anyhow::ensure!(
            self.max_ttl_seconds > 0,
            "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_MAX_TTL_SECONDS must be positive"
        );
        let signing_private_key_pem =
            validate_eddsa_private_key_pem(&self.signing_private_key_pem)?;
        let signing_public_key_pem = validate_eddsa_public_key_pem(&self.signing_public_key_pem)?;
        let workload_public_key_pem = validate_eddsa_public_key_pem(&self.workload_public_key_pem)?;
        let signing_key = EncodingKey::from_ed_pem(signing_private_key_pem.as_bytes())
            .context("invalid EdDSA signing private key")?;
        let workload_decoding_key = DecodingKey::from_ed_pem(workload_public_key_pem.as_bytes())
            .context("invalid EdDSA workload public key")?;
        anyhow::ensure!(
            !self.signing_kid.trim().is_empty(),
            "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_KID is required"
        );
        anyhow::ensure!(
            !self.issuer.trim().is_empty(),
            "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_ISSUER is required"
        );
        anyhow::ensure!(
            !self.audience.trim().is_empty(),
            "TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_AUDIENCE is required"
        );

        Ok(Arc::new(TraceUploadClaimIssuerState {
            signing_key,
            signing_kid: self.signing_kid.trim().to_string(),
            issuer: self.issuer.trim().to_string(),
            audience: self.audience.trim().to_string(),
            max_ttl_seconds: self.max_ttl_seconds,
            workload_decoding_key,
            workload_issuer: trim_optional(self.workload_issuer.clone()),
            workload_audience: trim_optional(self.workload_audience.clone()),
            signing_public_key_pem,
        }))
    }
}

struct TraceUploadClaimIssuerState {
    signing_key: EncodingKey,
    signing_kid: String,
    issuer: String,
    audience: String,
    max_ttl_seconds: i64,
    workload_decoding_key: DecodingKey,
    workload_issuer: Option<String>,
    workload_audience: Option<String>,
    signing_public_key_pem: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TraceUploadClaimRequest {
    schema_version: String,
    #[serde(default)]
    tenant_id: Option<String>,
    #[serde(default)]
    audience: Option<String>,
    #[serde(default)]
    trace_id: Option<Uuid>,
    #[serde(default)]
    submission_id: Option<Uuid>,
    #[serde(default)]
    consent_scopes: Vec<ConsentScope>,
    #[serde(default)]
    allowed_uses: Vec<TraceAllowedUse>,
    requested_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct TraceUploadClaimResponse {
    access_token: String,
    token_type: &'static str,
    expires_at: DateTime<Utc>,
    expires_in: i64,
}

#[derive(Debug, Deserialize)]
struct WorkloadClaims {
    #[serde(default)]
    sub: Option<String>,
    #[serde(default)]
    principal_ref: Option<String>,
    #[serde(default)]
    tenant_id: Option<String>,
    #[serde(default)]
    iss: Option<String>,
    #[serde(default)]
    aud: Option<serde_json::Value>,
    exp: i64,
    #[serde(default)]
    iat: Option<i64>,
    #[serde(default)]
    allowed_consent_scopes: Vec<ConsentScope>,
    #[serde(default)]
    allowed_uses: Vec<TraceAllowedUse>,
}

#[derive(Debug, Serialize)]
struct UploadClaimClaims {
    iss: String,
    aud: String,
    sub: String,
    principal_ref: String,
    tenant_id: String,
    role: &'static str,
    iat: i64,
    exp: i64,
    jti: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    trace_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    submission_id: Option<Uuid>,
    allowed_consent_scopes: Vec<ConsentScope>,
    allowed_uses: Vec<TraceAllowedUse>,
}

#[derive(Debug)]
struct IssuerError {
    status: StatusCode,
    message: &'static str,
}

impl IssuerError {
    fn bad_request(message: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
        }
    }

    fn forbidden(message: &'static str) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message,
        }
    }

    fn internal() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "failed to issue upload claim",
        }
    }
}

impl IntoResponse for IssuerError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

pub fn trace_upload_claim_issuer_router(
    config: TraceUploadClaimIssuerConfig,
) -> anyhow::Result<Router> {
    let state = config.build_state()?;
    Ok(Router::new()
        .route("/health", get(health_handler))
        .route(
            "/.well-known/trace-commons-ed25519-keyset.json",
            get(keyset_handler),
        )
        .route("/v1/trace-upload-claim", post(issue_claim_handler))
        .with_state(state))
}

pub async fn serve_trace_upload_claim_issuer(
    config: TraceUploadClaimIssuerConfig,
) -> anyhow::Result<()> {
    let bind = config.bind;
    let router = trace_upload_claim_issuer_router(config)?;
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("failed to bind Trace Commons upload claim issuer on {bind}"))?;
    axum::serve(listener, router)
        .await
        .context("Trace Commons upload claim issuer failed")
}

async fn health_handler() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

async fn keyset_handler(
    State(state): State<Arc<TraceUploadClaimIssuerState>>,
) -> Json<serde_json::Value> {
    Json(json!({
        "keys": [{
            "kid": state.signing_kid,
            "public_key_pem": state.signing_public_key_pem,
        }]
    }))
}

async fn issue_claim_handler(
    State(state): State<Arc<TraceUploadClaimIssuerState>>,
    headers: HeaderMap,
    Json(request): Json<TraceUploadClaimRequest>,
) -> Result<Json<TraceUploadClaimResponse>, IssuerError> {
    let workload = state.authenticate_workload(&headers)?;
    let response = state.issue_claim(&workload, request)?;
    Ok(Json(response))
}

impl TraceUploadClaimIssuerState {
    fn authenticate_workload(&self, headers: &HeaderMap) -> Result<WorkloadClaims, IssuerError> {
        let token = bearer_token(headers)?;
        let header = jsonwebtoken::decode_header(token)
            .map_err(|_| IssuerError::forbidden("invalid workload token"))?;
        if header.alg != Algorithm::EdDSA {
            return Err(IssuerError::forbidden("workload token must use EdDSA"));
        }

        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.validate_nbf = true;
        let mut required_claims = vec!["exp".to_string()];
        if let Some(issuer) = &self.workload_issuer {
            validation.set_issuer(&[issuer]);
            required_claims.push("iss".to_string());
        }
        if let Some(audience) = &self.workload_audience {
            validation.set_audience(&[audience]);
            required_claims.push("aud".to_string());
        } else {
            validation.validate_aud = false;
        }
        validation.set_required_spec_claims(&required_claims);

        let claims =
            jsonwebtoken::decode::<WorkloadClaims>(token, &self.workload_decoding_key, &validation)
                .map(|data| data.claims)
                .map_err(|error| match error.kind() {
                    JwtErrorKind::ExpiredSignature => {
                        IssuerError::forbidden("expired workload token")
                    }
                    JwtErrorKind::ImmatureSignature => {
                        IssuerError::forbidden("not-yet-valid workload token")
                    }
                    _ => IssuerError::forbidden("invalid workload token"),
                })?;
        self.validate_authenticated_workload_claims(&claims)?;
        Ok(claims)
    }

    fn validate_authenticated_workload_claims(
        &self,
        claims: &WorkloadClaims,
    ) -> Result<(), IssuerError> {
        if let Some(expected) = self.workload_issuer.as_deref()
            && claims.iss.as_deref() != Some(expected)
        {
            return Err(IssuerError::forbidden("invalid workload token"));
        }
        if let Some(expected) = self.workload_audience.as_deref()
            && !audience_claim_contains(claims.aud.as_ref(), expected)
        {
            return Err(IssuerError::forbidden("invalid workload token"));
        }
        let now = Utc::now().timestamp();
        if claims.exp <= now {
            return Err(IssuerError::forbidden("expired workload token"));
        }
        if let Some(iat) = claims.iat
            && iat > now + 60
        {
            return Err(IssuerError::forbidden("not-yet-valid workload token"));
        }
        Ok(())
    }

    fn issue_claim(
        &self,
        workload: &WorkloadClaims,
        request: TraceUploadClaimRequest,
    ) -> Result<TraceUploadClaimResponse, IssuerError> {
        if request.schema_version != TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION {
            return Err(IssuerError::bad_request(
                "unsupported request schema_version",
            ));
        }
        let now = Utc::now();
        if request.requested_at > now + Duration::minutes(5)
            || request.requested_at < now - Duration::minutes(15)
        {
            return Err(IssuerError::bad_request(
                "request requested_at is outside the accepted window",
            ));
        }
        if let Some(audience) = request.audience.as_deref().map(str::trim)
            && !audience.is_empty()
            && audience != self.audience
        {
            return Err(IssuerError::bad_request(
                "unsupported upload claim audience",
            ));
        }
        let tenant_id = normalized_required(
            request
                .tenant_id
                .as_deref()
                .or(workload.tenant_id.as_deref()),
            "tenant_id is required",
        )?;
        if let Some(workload_tenant) = workload.tenant_id.as_deref().map(str::trim)
            && !workload_tenant.is_empty()
            && workload_tenant != tenant_id
        {
            return Err(IssuerError::forbidden(
                "workload tenant does not match request",
            ));
        }
        enforce_subset(
            &request.consent_scopes,
            &workload.allowed_consent_scopes,
            "requested consent scopes exceed workload allowance",
        )?;
        enforce_subset(
            &request.allowed_uses,
            &workload.allowed_uses,
            "requested allowed uses exceed workload allowance",
        )?;

        let principal_ref = normalized_required(
            workload
                .principal_ref
                .as_deref()
                .or(workload.sub.as_deref()),
            "workload subject is required",
        )?;
        let expires_at = now
            .checked_add_signed(Duration::seconds(self.max_ttl_seconds))
            .ok_or_else(IssuerError::internal)?;
        let claims = UploadClaimClaims {
            iss: self.issuer.clone(),
            aud: self.audience.clone(),
            sub: principal_ref.clone(),
            principal_ref,
            tenant_id,
            role: "contributor",
            iat: now.timestamp(),
            exp: expires_at.timestamp(),
            jti: Uuid::new_v4().to_string(),
            trace_id: request.trace_id,
            submission_id: request.submission_id,
            allowed_consent_scopes: request.consent_scopes,
            allowed_uses: request.allowed_uses,
        };
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some(self.signing_kid.clone());
        let access_token = jsonwebtoken::encode(&header, &claims, &self.signing_key)
            .map_err(|_| IssuerError::internal())?;
        Ok(TraceUploadClaimResponse {
            access_token,
            token_type: "Bearer",
            expires_at,
            expires_in: self.max_ttl_seconds,
        })
    }
}

fn audience_claim_contains(audience: Option<&serde_json::Value>, expected: &str) -> bool {
    match audience {
        Some(serde_json::Value::String(audience)) => audience == expected,
        Some(serde_json::Value::Array(audiences)) => audiences
            .iter()
            .any(|audience| audience.as_str() == Some(expected)),
        _ => false,
    }
}

fn bearer_token(headers: &HeaderMap) -> Result<&str, IssuerError> {
    let value = headers
        .get(header::AUTHORIZATION)
        .ok_or_else(|| IssuerError::forbidden("missing workload token"))?
        .to_str()
        .map_err(|_| IssuerError::forbidden("invalid workload token"))?;
    value
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| IssuerError::forbidden("invalid workload token"))
}

fn enforce_subset<T: Ord>(
    requested: &[T],
    allowed: &[T],
    message: &'static str,
) -> Result<(), IssuerError> {
    if requested.is_empty() {
        return Ok(());
    }
    let allowed = allowed.iter().collect::<BTreeSet<_>>();
    if requested.iter().all(|item| allowed.contains(item)) {
        Ok(())
    } else {
        Err(IssuerError::forbidden(message))
    }
}

fn normalized_required(value: Option<&str>, message: &'static str) -> Result<String, IssuerError> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| IssuerError::bad_request(message))
}

fn validate_eddsa_private_key_pem(pem: &str) -> anyhow::Result<String> {
    let pem = pem.trim();
    anyhow::ensure!(!pem.contains("RSA"), "RSA keys are not supported");
    anyhow::ensure!(
        pem.starts_with("-----BEGIN PRIVATE KEY-----"),
        "EdDSA private key must be PKCS#8 PEM"
    );
    EncodingKey::from_ed_pem(pem.as_bytes()).context("invalid EdDSA private key")?;
    Ok(format!("{pem}\n"))
}

fn validate_eddsa_public_key_pem(pem: &str) -> anyhow::Result<String> {
    let pem = pem.trim();
    anyhow::ensure!(!pem.contains("RSA"), "RSA keys are not supported");
    anyhow::ensure!(
        pem.starts_with("-----BEGIN PUBLIC KEY-----"),
        "EdDSA public key must be SPKI PEM"
    );
    DecodingKey::from_ed_pem(pem.as_bytes()).context("invalid EdDSA public key")?;
    Ok(format!("{pem}\n"))
}

fn required_pem_or_file(
    inline_env: &'static str,
    file_env: &'static str,
) -> anyhow::Result<String> {
    let inline = optional_env(inline_env)?;
    let file = optional_env(file_env)?;
    match (inline, file) {
        (Some(_), Some(_)) => anyhow::bail!("{inline_env} and {file_env} cannot both be set"),
        (Some(value), None) => Ok(value),
        (None, Some(path)) => std::fs::read_to_string(PathBuf::from(path))
            .with_context(|| format!("failed to read {file_env}")),
        (None, None) => anyhow::bail!("{inline_env} or {file_env} is required"),
    }
}

fn required_env(name: &'static str) -> anyhow::Result<String> {
    optional_env(name)?.ok_or_else(|| anyhow::anyhow!("{name} is required"))
}

fn optional_env(name: &'static str) -> anyhow::Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) => {
            let trimmed = value.trim();
            Ok((!trimmed.is_empty()).then(|| trimmed.to_string()))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to read {name}")),
    }
}

fn trim_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode, header};
    use chrono::{Duration, Utc};
    use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
    use serde_json::json;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::trace_contribution::{ConsentScope, TraceAllowedUse};

    const TEST_EDDSA_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIAGfN68ko7YyCGJMb3lHVwTn5aiUtbIsAclIx/lX0p2R\n-----END PRIVATE KEY-----\n";
    const TEST_EDDSA_PUBLIC_KEY_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAMnniSMeHZrdoe3gkL7ZeHmG7vAg65c5TqaBd71B2qDw=\n-----END PUBLIC KEY-----\n";
    const WORKLOAD_EDDSA_PRIVATE_KEY_PEM: &str = TEST_EDDSA_PRIVATE_KEY_PEM;
    const WORKLOAD_EDDSA_PUBLIC_KEY_PEM: &str = TEST_EDDSA_PUBLIC_KEY_PEM;

    fn test_config() -> TraceUploadClaimIssuerConfig {
        TraceUploadClaimIssuerConfig {
            bind: "127.0.0.1:0".parse().expect("bind parses"),
            signing_private_key_pem: TEST_EDDSA_PRIVATE_KEY_PEM.to_string(),
            signing_public_key_pem: TEST_EDDSA_PUBLIC_KEY_PEM.to_string(),
            signing_kid: "issuer-key-1".to_string(),
            issuer: "trace-commons-upload-issuer".to_string(),
            audience: "trace-commons-upload".to_string(),
            max_ttl_seconds: 300,
            workload_public_key_pem: WORKLOAD_EDDSA_PUBLIC_KEY_PEM.to_string(),
            workload_issuer: Some("workload-issuer".to_string()),
            workload_audience: Some("trace-claim-issuer".to_string()),
        }
    }

    fn workload_token(issuer: &str, audience: &str) -> String {
        let now = Utc::now();
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some("workload-key-1".to_string());
        jsonwebtoken::encode(
            &header,
            &json!({
                "sub": "principal:agent-1",
                "principal_ref": "principal:agent-1",
                "tenant_id": "tenant-a",
                "iss": issuer,
                "aud": audience,
                "iat": now.timestamp(),
                "exp": (now + Duration::minutes(5)).timestamp(),
                "allowed_consent_scopes": ["debugging_evaluation", "benchmark_only"],
                "allowed_uses": ["debugging", "evaluation"],
            }),
            &EncodingKey::from_ed_pem(WORKLOAD_EDDSA_PRIVATE_KEY_PEM.as_bytes())
                .expect("workload key parses"),
        )
        .expect("workload token signs")
    }

    fn claim_request() -> serde_json::Value {
        json!({
            "schema_version": TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION,
            "tenant_id": "tenant-a",
            "audience": "trace-commons-upload",
            "trace_id": Uuid::new_v4(),
            "submission_id": Uuid::new_v4(),
            "consent_scopes": ["debugging_evaluation"],
            "allowed_uses": ["debugging"],
            "requested_at": Utc::now(),
        })
    }

    async fn post_claim(
        config: TraceUploadClaimIssuerConfig,
        token: String,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let router = trace_upload_claim_issuer_router(config).expect("router builds");
        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/trace-upload-claim")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(body.to_string()))
                    .expect("request builds"),
            )
            .await
            .expect("request completes");
        let status = response.status();
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body reads");
        let json = serde_json::from_slice(&body).expect("json response");
        (status, json)
    }

    #[tokio::test]
    async fn eddsa_only_issue_success_returns_bounded_upload_claim() {
        let (status, body) = post_claim(
            test_config(),
            workload_token("workload-issuer", "trace-claim-issuer"),
            claim_request(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["token_type"], "Bearer");
        assert!(body["expires_in"].as_i64().expect("expires_in") <= 300);

        let token = body["access_token"].as_str().expect("access token");
        let header = jsonwebtoken::decode_header(token).expect("issuer token header");
        assert_eq!(header.alg, Algorithm::EdDSA);
        assert_eq!(header.kid.as_deref(), Some("issuer-key-1"));

        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_issuer(&["trace-commons-upload-issuer"]);
        validation.set_audience(&["trace-commons-upload"]);
        let claims = jsonwebtoken::decode::<serde_json::Value>(
            token,
            &DecodingKey::from_ed_pem(TEST_EDDSA_PUBLIC_KEY_PEM.as_bytes())
                .expect("issuer public key parses"),
            &validation,
        )
        .expect("issuer token verifies")
        .claims;
        assert_eq!(claims["tenant_id"], "tenant-a");
        assert_eq!(claims["role"], "contributor");
        assert_eq!(claims["sub"], "principal:agent-1");
        assert_eq!(claims["principal_ref"], "principal:agent-1");
        assert_eq!(
            claims["allowed_consent_scopes"],
            json!(["debugging_evaluation"])
        );
        assert_eq!(claims["allowed_uses"], json!(["debugging"]));
        assert!(claims["jti"].as_str().is_some_and(|jti| !jti.is_empty()));
    }

    #[tokio::test]
    async fn wrong_workload_audience_or_issuer_is_rejected() {
        let (status, _) = post_claim(
            test_config(),
            workload_token("wrong-issuer", "trace-claim-issuer"),
            claim_request(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _) = post_claim(
            test_config(),
            workload_token("workload-issuer", "wrong-audience"),
            claim_request(),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn no_rsa_or_generic_jwks_material_is_accepted_or_exposed() {
        assert!(
            TraceUploadClaimIssuerConfig {
                signing_private_key_pem:
                    "-----BEGIN RSA PRIVATE KEY-----\nredacted\n-----END RSA PRIVATE KEY-----"
                        .to_string(),
                ..test_config()
            }
            .build_state()
            .is_err()
        );

        let router = trace_upload_claim_issuer_router(test_config()).expect("router builds");
        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/.well-known/trace-commons-ed25519-keyset.json")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("request completes");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body reads");
        let text = String::from_utf8(body.to_vec()).expect("utf8");
        assert!(text.contains("\"public_key_pem\""));
        assert!(text.contains("BEGIN PUBLIC KEY"));
        assert!(!text.contains("\"kty\""));
        assert!(!text.contains("\"crv\""));
        assert!(!text.contains("\"x\""));
        assert!(!text.contains("RSA"));
        assert!(!text.contains("BEGIN PRIVATE KEY"));
    }

    #[test]
    fn rejects_requests_exceeding_workload_allowances() {
        let state = test_config().build_state().expect("state builds");
        let workload = WorkloadClaims {
            sub: Some("principal:agent-1".to_string()),
            principal_ref: None,
            tenant_id: Some("tenant-a".to_string()),
            iss: None,
            aud: None,
            exp: Utc::now().timestamp() + 60,
            iat: Some(Utc::now().timestamp()),
            allowed_consent_scopes: vec![ConsentScope::DebuggingEvaluation],
            allowed_uses: vec![TraceAllowedUse::Debugging],
        };
        let request = TraceUploadClaimRequest {
            schema_version: TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION.to_string(),
            tenant_id: Some("tenant-a".to_string()),
            audience: Some("trace-commons-upload".to_string()),
            trace_id: None,
            submission_id: None,
            consent_scopes: vec![ConsentScope::ModelTraining],
            allowed_uses: vec![TraceAllowedUse::ModelTraining],
            requested_at: Utc::now(),
        };
        assert!(state.issue_claim(&workload, request).is_err());
    }
}
