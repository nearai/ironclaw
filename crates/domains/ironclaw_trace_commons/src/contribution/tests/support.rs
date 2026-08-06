//! Shared fixtures for the `contribution` test modules: privacy-filter and
//! credential-provider fakes, env guards, and record builders.

use std::collections::BTreeMap;
use std::path::PathBuf;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use ironclaw_llm::recording::{TraceFile, TraceResponse};

use crate::contribution::*;
use base64::Engine;
use ironclaw_llm::recording::{TraceStep, TraceToolCall};

pub(super) struct FakePrivacyFilterAdapter;
#[async_trait]
impl PrivacyFilterAdapter for FakePrivacyFilterAdapter {
    async fn redact_text(
        &self,
        text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
        if !text.contains("Alice") {
            return Ok(None);
        }
        let mut report = RedactionReport::default();
        report.increment("privacy_filter:private_person");
        report.add_pii_label("private_person");
        Ok(Some(SafePrivacyFilterRedaction {
            redacted_text: text.replace("Alice", "<PRIVATE_PERSON_1>"),
            summary: SafePrivacyFilterSummary {
                schema_version: 1,
                output_mode: "redacted_text_only".to_string(),
                span_count: 1,
                by_label: BTreeMap::from([("private_person".to_string(), 1)]),
                decoded_mismatch: false,
            },
            report,
        }))
    }
}
pub(super) struct CanaryPrivacyFilterAdapter;
#[async_trait]
impl PrivacyFilterAdapter for CanaryPrivacyFilterAdapter {
    async fn redact_text(
        &self,
        text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
        let values = synthetic_privacy_filter_canary_values();
        let mut redacted = text.to_string();
        for (index, value) in values.iter().enumerate() {
            redacted = redacted.replace(value, &format!("<CANARY_REDACTED_{}>", index + 1));
        }
        let output = serde_json::json!({
            "schema_version": 1,
            "text": text,
            "redacted_text": redacted,
            "detected_spans": [
                {"label": "private_email", "text": values[0]},
                {"label": "secret", "text": values[1]},
                {"label": "local_path", "text": values[2]}
            ]
        });
        safe_privacy_filter_redaction_from_output(&output).map(Some)
    }
}
pub(super) struct FailingPrivacyFilterAdapter;
#[async_trait]
impl PrivacyFilterAdapter for FailingPrivacyFilterAdapter {
    async fn redact_text(
        &self,
        _text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
        Err(TraceContributionError::RedactionFailed {
            reason: "sidecar stderr mentioned tc_canary_secret_0123456789abcdef".to_string(),
        })
    }
}
/// Sets a **real** process environment variable for the life of the guard and
/// restores the previous value on drop.
///
/// The process environment is global, so the guard holds
/// `ironclaw_common::env_helpers::lock_env()` for its whole lifetime: without
/// it, two tests mutating the environment on different threads race, which is
/// undefined behavior on Rust 1.82+ regardless of whether they name the same
/// variable. The lock field is declared last so it is released *after*
/// `Drop::drop` has restored the value.
///
/// The real process environment is required rather than
/// `env_helpers::set_runtime_env`'s overlay: the sidecar isolation test needs
/// a value a **child process** would inherit, to prove
/// `CommandPrivacyFilterAdapter` clears it.
pub(super) struct EnvVarRestore {
    pub(super) name: &'static str,
    pub(super) previous: Option<String>,
    _env_lock: std::sync::MutexGuard<'static, ()>,
}
impl EnvVarRestore {
    pub(super) fn set(name: &'static str, value: &str) -> Self {
        let _env_lock = ironclaw_common::env_helpers::lock_env();
        let previous = std::env::var(name).ok();
        // SAFETY: serialized by the env lock held for this guard's lifetime,
        // and restored in Drop before that lock is released.
        unsafe {
            std::env::set_var(name, value);
        }
        Self {
            name,
            previous,
            _env_lock,
        }
    }
}
impl Drop for EnvVarRestore {
    fn drop(&mut self) {
        // The `lock_env()` guard taken in `set` is still held for this whole
        // body — `_env_lock` is a field, so it is released only after `drop`
        // returns.
        //
        // SAFETY: that lock serializes this restore against every other test
        // that mutates the process environment.
        unsafe {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var(self.name, previous);
            } else {
                std::env::remove_var(self.name);
            }
        }
    }
}
pub(super) struct RefreshingTestUploadCredentialProvider {
    pub(super) current: std::sync::Mutex<String>,
    pub(super) fresh: String,
}
impl RefreshingTestUploadCredentialProvider {
    pub(super) fn new(stale: &str, fresh: &str) -> Self {
        Self {
            current: std::sync::Mutex::new(stale.to_string()),
            fresh: fresh.to_string(),
        }
    }
}
#[async_trait]
impl TraceUploadCredentialProvider for RefreshingTestUploadCredentialProvider {
    async fn bearer_token(
        &self,
        _policy: &StandingTraceContributionPolicy,
        _context: &TraceUploadClaimContext,
        force_refresh: bool,
    ) -> anyhow::Result<String> {
        let mut current = self.current.lock().expect("test provider lock");
        if force_refresh {
            *current = self.fresh.clone();
        }
        Ok(current.clone())
    }
}
/// Records the (subject, scope_dir) of every claim context it is asked to
/// mint for, so tests can assert the credential context that status sync
/// actually used.
#[derive(Default)]
pub(super) struct CapturingUploadCredentialProvider {
    pub(super) contexts: std::sync::Mutex<Vec<(Option<String>, Option<PathBuf>)>>,
}
#[async_trait]
impl TraceUploadCredentialProvider for CapturingUploadCredentialProvider {
    async fn bearer_token(
        &self,
        _policy: &StandingTraceContributionPolicy,
        context: &TraceUploadClaimContext,
        _force_refresh: bool,
    ) -> anyhow::Result<String> {
        self.contexts
            .lock()
            .expect("capturing provider lock")
            .push((context.subject.clone(), context.scope_dir.clone()));
        Ok("captured-token".to_string())
    }
}
pub(super) struct FailingTestUploadCredentialProvider {
    pub(super) kind: std::io::ErrorKind,
}
#[async_trait]
impl TraceUploadCredentialProvider for FailingTestUploadCredentialProvider {
    async fn bearer_token(
        &self,
        _policy: &StandingTraceContributionPolicy,
        _context: &TraceUploadClaimContext,
        _force_refresh: bool,
    ) -> anyhow::Result<String> {
        Err(std::io::Error::new(
            self.kind,
            "credential provider failed while using super-secret-token",
        )
        .into())
    }
}
pub(super) fn sample_trace() -> TraceFile {
    TraceFile {
        model_name: "test-model".to_string(),
        usage: None,
        memory_snapshot: Vec::new(),
        http_exchanges: Vec::new(),
        steps: vec![
            TraceStep {
                request_hint: None,
                response: TraceResponse::UserInput {
                    content: "Email alice@example.com about /Users/alice/project/secrets.txt"
                        .to_string(),
                },
                expected_tool_results: Vec::new(),
            },
            TraceStep {
                request_hint: None,
                response: TraceResponse::ToolCalls {
                    tool_calls: vec![TraceToolCall {
                        id: "call_1".to_string(),
                        name: "http".to_string(),
                        arguments: serde_json::json!({
                            "url": "https://api.example.com",
                            "Authorization": "Bearer abcdefghijklmnopqrstuvwxyz123456",
                            "path": "/Users/alice/project/secrets.txt"
                        }),
                    }],
                    input_tokens: 10,
                    output_tokens: 3,
                },
                expected_tool_results: Vec::new(),
            },
        ],
    }
}
pub(super) fn submitted_credit_record(
    credit_points_pending: f32,
    credit_points_final: Option<f32>,
    last_credit_notice_at: Option<DateTime<Utc>>,
    credit_explanation: Vec<String>,
) -> NodeTraceSubmissionRecord {
    NodeTraceSubmissionRecord {
        submission_id: Uuid::new_v4(),
        trace_id: Uuid::new_v4(),
        endpoint: Some("https://trace.example.com/v1/traces".to_string()),
        status: NodeTraceSubmissionStatus::Submitted,
        server_status: Some("accepted".to_string()),
        submitted_at: Some(Utc::now()),
        revoked_at: None,
        privacy_risk: "low".to_string(),
        redaction_counts: BTreeMap::new(),
        credit_points_pending,
        credit_points_final,
        credit_explanation,
        credit_events: Vec::new(),
        history: Vec::new(),
        last_credit_notice_at,
        credit_notice_state: TraceCreditNoticeState::default(),
    }
}
pub(super) fn test_jwt_with_header(header: serde_json::Value) -> String {
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    format!(
        "{}.{}.signature",
        engine.encode(header.to_string().as_bytes()),
        engine.encode(b"{}")
    )
}
pub(super) fn write_policy_at(
    base: &std::path::Path,
    scope: Option<&str>,
    policy: &StandingTraceContributionPolicy,
) {
    write_trace_policy_for_scope_at(base, scope, policy).expect("write_policy_at");
}
/// Minimal reqwest-backed ContributionHttpSink for use in unit tests that
/// need to exercise the sink path against a local mock server.
pub(super) struct ReqwestContributionSink;
#[async_trait]
impl ContributionHttpSink for ReqwestContributionSink {
    async fn execute(
        &self,
        req: ContributionHttpRequest,
    ) -> Result<ContributionHttpResponse, ContributionHttpError> {
        let method = match req.method {
            ContributionHttpMethod::Get => reqwest::Method::GET,
            ContributionHttpMethod::Post => reqwest::Method::POST,
            ContributionHttpMethod::Put => reqwest::Method::PUT,
            ContributionHttpMethod::Delete => reqwest::Method::DELETE,
        };
        let client = reqwest::Client::new();
        let mut builder = client.request(method, &req.url);
        if let Some(token) = req.bearer_token {
            builder = builder.bearer_auth(token);
        }
        if let Some(body) = req.json_body {
            builder = builder
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body);
        }
        let response = builder
            .send()
            .await
            .map_err(|e| ContributionHttpError::new(e.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .bytes()
            .await
            .map_err(|e| ContributionHttpError::new(e.to_string()))?
            .to_vec();
        Ok(ContributionHttpResponse { status, body })
    }
}
/// Sink wrapper that records every request URL it executes. Used to pin
/// the egress invariant: on the agent (sink) path, EVERY network call —
/// including the upload-claim mint — must route through the sink, not a
/// direct reqwest client.
pub(super) struct RecordingSink {
    pub(super) inner: ReqwestContributionSink,
    pub(super) urls: std::sync::Mutex<Vec<String>>,
}
impl RecordingSink {
    pub(super) fn new() -> Self {
        Self {
            inner: ReqwestContributionSink,
            urls: std::sync::Mutex::new(Vec::new()),
        }
    }
}
#[async_trait]
impl ContributionHttpSink for RecordingSink {
    async fn execute(
        &self,
        req: ContributionHttpRequest,
    ) -> Result<ContributionHttpResponse, ContributionHttpError> {
        self.urls
            .lock()
            .expect("recording sink lock")
            .push(req.url.clone());
        self.inner.execute(req).await
    }
}
/// Write an instance policy (scope `None`) and promote a device key at the
/// instance scope dir, so `resolve_trace_credentials_at` returns a per-user
/// pseudonymous subject and DeviceKey auth can sign without a network call.
/// Returns nothing — the caller reads back via the resolver.
pub(super) fn enroll_instance_with_device_key(base: &std::path::Path, addr: std::net::SocketAddr) {
    let policy = StandingTraceContributionPolicy {
        enabled: true,
        auth_mode: TraceUploadAuthMode::DeviceKey,
        upload_token_issuer_url: Some(format!("http://{addr}/v1/trace-upload-claim")),
        upload_token_issuer_allowed_hosts: std::collections::BTreeSet::from([
            "127.0.0.1".to_string()
        ]),
        upload_token_tenant_id: Some("tenant-dev".to_string()),
        upload_token_audience: Some("trace-commons-ingest".to_string()),
        ingestion_endpoint: Some(format!("http://{addr}/v1/traces")),
        ..Default::default()
    };
    write_trace_policy_for_scope_at(base, None, &policy).expect("instance policy writes");
    let instance_dir = trace_contribution_dir_for_scope_at(base, None);
    let pending =
        crate::onboarding::DeviceKeypair::load_or_generate_pending(&instance_dir, "testhash")
            .unwrap();
    pending.promote(&instance_dir, "tenant-dev").unwrap();
}
/// Helper: instance-enroll a tempdir against a mock whose
/// `/v1/account/login-links` returns `link_url`, then mint via the direct
/// path. Pins the origin-anchoring contract for hostile response URLs.
pub(super) async fn mint_login_link_with_response_url(
    link_url: &str,
) -> (Result<AccountLoginLink, AccountLoginLinkError>, String) {
    let claim_jwt = test_jwt_with_header(serde_json::json!({"alg": "EdDSA", "kid": "k1"}));
    let claim_jwt_for_mock = claim_jwt.clone();
    let link_url = link_url.to_string();
    let app = axum::Router::new()
        .route(
            "/v1/trace-upload-claim",
            axum::routing::post(move || {
                let jwt = claim_jwt_for_mock.clone();
                async move {
                    axum::Json(serde_json::json!({
                        "access_token": jwt, "token_type": "Bearer", "expires_in": 300
                    }))
                }
            }),
        )
        .route(
            "/v1/account/login-links",
            axum::routing::post(move || {
                let url = link_url.clone();
                async move {
                    axum::Json(serde_json::json!({
                        "account_id": "11111111-1111-1111-1111-111111111111",
                        "url": url
                    }))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let base = tempfile::tempdir().unwrap();
    enroll_instance_with_device_key(base.path(), addr);
    let result = mint_account_login_link_direct(base.path(), "tenant-dev", "alice").await;
    (result, format!("http://{addr}"))
}
/// Helper: enroll an instance scope at `base` against a mock that serves the
/// claim issuer plus `/v1/account/traces` returning `status`/`body`. Returns
/// the results of BOTH fetch paths — the sink-backed
/// `fetch_account_traces_inner` (agent path) and the direct
/// `fetch_account_traces_direct` (WebUI/CLI path, pinned reqwest client) —
/// so status-handling regressions in either path are caught.
pub(super) async fn fetch_account_traces_with_status(
    status: axum::http::StatusCode,
    body: serde_json::Value,
) -> (
    anyhow::Result<Vec<AccountTraceItem>>,
    anyhow::Result<Vec<AccountTraceItem>>,
) {
    let claim_jwt = test_jwt_with_header(serde_json::json!({"alg": "EdDSA", "kid": "k1"}));
    let claim_jwt_for_mock = claim_jwt.clone();
    let app = axum::Router::new()
        .route(
            "/v1/trace-upload-claim",
            axum::routing::post(move || {
                let jwt = claim_jwt_for_mock.clone();
                async move {
                    axum::Json(serde_json::json!({
                        "access_token": jwt, "token_type": "Bearer", "expires_in": 300
                    }))
                }
            }),
        )
        .route(
            "/v1/account/traces",
            axum::routing::get(move || {
                let body = body.clone();
                async move { (status, axum::Json(body)) }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let base = tempfile::tempdir().unwrap();
    let policy = StandingTraceContributionPolicy {
        enabled: true,
        auth_mode: TraceUploadAuthMode::DeviceKey,
        upload_token_issuer_url: Some(format!("http://{addr}/v1/trace-upload-claim")),
        upload_token_issuer_allowed_hosts: std::collections::BTreeSet::from([
            "127.0.0.1".to_string()
        ]),
        upload_token_tenant_id: Some("tenant-dev".to_string()),
        upload_token_audience: Some("trace-commons-ingest".to_string()),
        ..Default::default()
    };
    write_trace_policy_for_scope_at(base.path(), None, &policy).unwrap();
    let instance_dir = trace_contribution_dir_for_scope_at(base.path(), None);
    crate::onboarding::DeviceKeypair::load_or_generate_pending(&instance_dir, "h")
        .unwrap()
        .promote(&instance_dir, "tenant-dev")
        .unwrap();

    let sink = ReqwestContributionSink;
    let via_sink =
        fetch_account_traces_inner(base.path(), "tenant-dev", "alice", None, &sink).await;
    let direct = fetch_account_traces_direct(base.path(), "tenant-dev", "alice", None).await;
    (via_sink, direct)
}
