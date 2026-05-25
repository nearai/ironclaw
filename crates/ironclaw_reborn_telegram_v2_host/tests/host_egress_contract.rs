//! Contract tests for [`HostMediatedTelegramEgress`].
//!
//! The shim is the production [`ProtocolHttpEgress`] for Telegram v2 and
//! replaced the bespoke `TelegramHttpEgress` that previously owned its own
//! reqwest client + credential resolver. Those bespoke tests went away with
//! the old code; this file is the regression net for the new shim.
//!
//! Strategy: drive `HostMediatedTelegramEgress::send` through a recording
//! fake [`RuntimeHttpEgress`] so we can assert the URL, method, headers,
//! body, credential-injection plan, and network policy the host-api
//! receives — exactly the surface a regression in URL construction,
//! credential routing, or method translation would corrupt.
//!
//! Test-through-the-caller (`.claude/rules/testing.md`): each branch is
//! exercised via the public `ProtocolHttpEgress::send` entry point rather
//! than a helper on the impl, so a wrapper that silently drops one of the
//! inputs (a header, the body, the per-request credential override) is
//! caught.

use std::sync::{Arc, Mutex};

use ironclaw_host_api::{
    AgentId, CapabilityId, InvocationId, ProjectId, ResourceScope, RuntimeCredentialTarget,
    RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse,
    TenantId, UserId,
};
use ironclaw_product_adapters::{
    DeclaredEgressHost, DeclaredEgressTarget, EgressCredentialHandle, EgressMethod, EgressPath,
    EgressRequest, ProtocolHttpEgress, ProtocolHttpEgressError,
};
use ironclaw_reborn_telegram_v2_host::host_egress::HostMediatedTelegramEgress;

/// Recording fake for [`RuntimeHttpEgress`]. Captures every request the
/// host shim hands down so tests can assert URL/method/headers/body and
/// the credential-injection plan; configurable response lets tests pin
/// success and each failure shape.
struct RecordingRuntimeEgress {
    response: Mutex<Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError>>,
    requests: Arc<Mutex<Vec<RuntimeHttpEgressRequest>>>,
}

impl RecordingRuntimeEgress {
    fn with_response(response: RuntimeHttpEgressResponse) -> Self {
        Self {
            response: Mutex::new(Ok(response)),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Construct a fake that returns the given runtime error on every
    /// `execute()`. Used to pin the `map_runtime_error` matrix from the
    /// caller side — verifying the adapter-visible variant each runtime
    /// error class translates into, since the retry classifier in
    /// `ironclaw_product_adapters::error` branches on the variant.
    fn with_error(error: RuntimeHttpEgressError) -> Self {
        Self {
            response: Mutex::new(Err(error)),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl RuntimeHttpEgress for RecordingRuntimeEgress {
    fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.requests.lock().unwrap().push(request);
        let mut guard = self.response.lock().unwrap();
        match &mut *guard {
            Ok(response) => Ok(response.clone()),
            Err(error) => Err(error.clone()),
        }
    }
}

fn telegram_host() -> DeclaredEgressHost {
    DeclaredEgressHost::new("api.telegram.org").unwrap()
}

fn declared_handle() -> EgressCredentialHandle {
    EgressCredentialHandle::new("telegram_bot_token_default").unwrap()
}

fn telegram_target_with_default_handle() -> DeclaredEgressTarget {
    DeclaredEgressTarget::new(telegram_host(), Some(declared_handle()))
}

fn telegram_target_without_handle() -> DeclaredEgressTarget {
    DeclaredEgressTarget::new(telegram_host(), None)
}

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("default").unwrap(),
        user_id: UserId::new("test-user").unwrap(),
        agent_id: Some(AgentId::new("default").unwrap()),
        project_id: Some(ProjectId::new("bootstrap").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn sample_capability_id() -> CapabilityId {
    CapabilityId::new("telegram.send_message").unwrap()
}

fn ok_response() -> RuntimeHttpEgressResponse {
    RuntimeHttpEgressResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        request_bytes: 32,
        response_bytes: 11,
        redaction_applied: false,
    }
}

fn build_shim(
    egress: Arc<RecordingRuntimeEgress>,
    target: DeclaredEgressTarget,
) -> HostMediatedTelegramEgress<RecordingRuntimeEgress> {
    HostMediatedTelegramEgress::new(egress, vec![target], sample_scope(), sample_capability_id())
        .expect("Telegram host is declared, shim should construct")
}

fn telegram_send_message_request() -> EgressRequest {
    EgressRequest::new(
        telegram_host(),
        EgressMethod::post(),
        EgressPath::new("/sendMessage").unwrap(),
    )
    .with_body(br#"{"chat_id":42,"text":"hi"}"#.to_vec())
}

/// Happy path: a Telegram POST flows through the shim and the host-api
/// receives a request with the placeholder URL, the right method, the
/// body verbatim, and a UrlPath credential-injection plan pointing at the
/// declared default handle.
#[tokio::test]
async fn host_egress_send_translates_request_into_runtime_egress() {
    let runtime = Arc::new(RecordingRuntimeEgress::with_response(ok_response()));
    let shim = build_shim(Arc::clone(&runtime), telegram_target_with_default_handle());

    let response = shim
        .send(telegram_send_message_request())
        .await
        .expect("host-mediated egress should succeed");
    assert_eq!(response.status(), 200);
    assert_eq!(response.body(), br#"{"ok":true}"#);

    let captured = runtime.requests();
    assert_eq!(captured.len(), 1);
    let captured = &captured[0];

    // URL: scheme + host + bot<PLACEHOLDER> + adapter path.
    assert_eq!(
        captured.url, "https://api.telegram.org/bot{telegram_bot_token}/sendMessage",
        "URL must embed the credential placeholder, not the resolved token",
    );

    // Method translated to NetworkMethod::Post.
    assert!(
        matches!(captured.method, ironclaw_host_api::NetworkMethod::Post),
        "POST should translate to NetworkMethod::Post, got {:?}",
        captured.method
    );

    // Body forwarded byte-for-byte.
    assert_eq!(captured.body, br#"{"chat_id":42,"text":"hi"}"#);

    // Credential-injection plan: exactly one injection, UrlPath target,
    // placeholder matches the URL, handle resolves to the declared default.
    assert_eq!(captured.credential_injections.len(), 1);
    let injection = &captured.credential_injections[0];
    assert_eq!(injection.handle.as_str(), "telegram_bot_token_default");
    assert!(injection.required);
    match &injection.target {
        RuntimeCredentialTarget::UrlPath { placeholder } => {
            assert_eq!(placeholder, "{telegram_bot_token}");
        }
        other => panic!("expected UrlPath credential target, got {other:?}"),
    }

    // Capability + scope stamped from the shim.
    assert_eq!(captured.capability_id.as_str(), "telegram.send_message");
    assert_eq!(captured.scope.user_id.as_str(), "test-user");

    // Network policy: api.telegram.org is in the allowlist, private IPs
    // denied, and no max-egress override.
    assert_eq!(captured.network_policy.allowed_targets.len(), 1);
    assert_eq!(
        captured.network_policy.allowed_targets[0].host_pattern,
        "api.telegram.org"
    );
    assert!(captured.network_policy.deny_private_ip_ranges);
}

/// A request to a host not in the declared list is rejected with the
/// typed `UndeclaredHost` error *before* any host-api call. The shim
/// short-circuits here so the adapter sees the precise contract error
/// instead of the opaque `Network`/`Request` variants the host-api
/// collapses non-allowlisted hosts into.
#[tokio::test]
async fn host_egress_rejects_undeclared_host_before_runtime_call() {
    let runtime = Arc::new(RecordingRuntimeEgress::with_response(ok_response()));
    // Shim declares only api.telegram.org; build the request against a
    // *different* host.
    let other_host = DeclaredEgressHost::new("evil.example").unwrap();
    let shim = build_shim(Arc::clone(&runtime), telegram_target_with_default_handle());

    let request = EgressRequest::new(
        other_host,
        EgressMethod::post(),
        EgressPath::new("/leak").unwrap(),
    )
    .with_body(b"hi".to_vec());

    let error = shim
        .send(request)
        .await
        .expect_err("undeclared host must fail closed");

    match error {
        ProtocolHttpEgressError::UndeclaredHost { host } => {
            assert_eq!(host, "evil.example");
        }
        other => panic!("expected UndeclaredHost, got {other:?}"),
    }
    assert!(
        runtime.requests().is_empty(),
        "runtime egress must not be called for undeclared hosts",
    );
}

/// When neither the declared default nor the per-request override supplies
/// a credential handle, the shim returns `UnauthorizedCredentialHandle`
/// rather than silently sending an unauthenticated request.
#[tokio::test]
async fn host_egress_rejects_request_without_credential_handle() {
    let runtime = Arc::new(RecordingRuntimeEgress::with_response(ok_response()));
    // Declared target has no default handle.
    let shim = build_shim(Arc::clone(&runtime), telegram_target_without_handle());

    let error = shim
        .send(telegram_send_message_request())
        .await
        .expect_err("missing credential handle must fail closed");

    match error {
        ProtocolHttpEgressError::UnauthorizedCredentialHandle { handle } => {
            assert_eq!(handle, "(missing)");
        }
        other => panic!("expected UnauthorizedCredentialHandle, got {other:?}"),
    }
    assert!(
        runtime.requests().is_empty(),
        "runtime egress must not be called when no credential handle is resolvable",
    );
}

/// Per-request credential handle takes precedence over the declared
/// default. This is the only way an adapter switches between multiple
/// configured bot tokens for the same host, so the override path is
/// load-bearing for multi-tenant deployments.
#[tokio::test]
async fn host_egress_per_request_credential_overrides_declared_default() {
    let runtime = Arc::new(RecordingRuntimeEgress::with_response(ok_response()));
    let shim = build_shim(Arc::clone(&runtime), telegram_target_with_default_handle());

    let override_handle = EgressCredentialHandle::new("telegram_bot_token_per_request").unwrap();
    let request = telegram_send_message_request().with_credential_handle(Some(override_handle));

    shim.send(request)
        .await
        .expect("per-request credential override should still succeed");

    let captured = runtime.requests();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].credential_injections.len(), 1);
    assert_eq!(
        captured[0].credential_injections[0].handle.as_str(),
        "telegram_bot_token_per_request",
        "per-request handle must override the declared default",
    );
}

/// HTTP methods the host shim does not yet translate (PUT/PATCH/DELETE)
/// surface as `PolicyDenied`. The Telegram Bot API only uses POST and
/// GET, so this branch is defence-in-depth — a future adapter change
/// that emits PUT must be a conscious extension here, not a silent
/// promotion to the wire.
#[tokio::test]
async fn host_egress_rejects_unsupported_http_method() {
    let runtime = Arc::new(RecordingRuntimeEgress::with_response(ok_response()));
    let shim = build_shim(Arc::clone(&runtime), telegram_target_with_default_handle());

    // PUT is accepted by EgressMethod (constructor allow-list includes
    // GET/POST/PUT/PATCH/DELETE) but the host shim's translation matrix
    // only handles POST + GET.
    let put_method = EgressMethod::new("PUT").unwrap();
    let request = EgressRequest::new(
        telegram_host(),
        put_method,
        EgressPath::new("/sendMessage").unwrap(),
    )
    .with_body(b"{}".to_vec());

    let error = shim
        .send(request)
        .await
        .expect_err("unsupported method must fail closed");

    // The reason carries `RedactedString` which intentionally renders as
    // `<redacted>` to callers; we lock in only the variant so the adapter
    // retry classifier (which branches on the variant, not the message)
    // sees a permanent denial rather than a retryable Network error.
    match error {
        ProtocolHttpEgressError::PolicyDenied { .. } => {}
        other => panic!("expected PolicyDenied, got {other:?}"),
    }
    assert!(
        runtime.requests().is_empty(),
        "runtime egress must not be called for unsupported methods",
    );
}

/// Construction-time fail-closed: an empty declared list refuses to
/// build the shim. The intent summary called this out as an explicit
/// safety check; a regression that flipped the conditional would
/// allow the policy fabric to accept anything.
#[test]
fn host_egress_construction_rejects_empty_declared_list() {
    let runtime = Arc::new(RecordingRuntimeEgress::with_response(ok_response()));

    let result = HostMediatedTelegramEgress::new(
        runtime,
        Vec::new(),
        sample_scope(),
        sample_capability_id(),
    );
    let error = match result {
        Ok(_) => panic!("empty declared list must fail closed"),
        Err(error) => error,
    };
    let rendered = error.to_string();
    assert!(
        rendered.contains("open network policy") || rendered.contains("declared egress"),
        "construction error should explain the open-policy refusal, got: {rendered}",
    );
}

// ----------------------------------------------------------------------------
// map_runtime_error retry-classification matrix
//
// The shim translates `RuntimeHttpEgressError` into `ProtocolHttpEgressError`,
// and the adapter's `is_retryable` classifier branches on the variant. Pre-fix
// the mapping made permanent runtime failures (leak detector, credential
// store) look retryable, or stuffed free-text reason strings into typed-id
// fields. These tests lock the contract: Network → retryable Network,
// everything else → permanent PolicyDenied (with a stable, short reason).
// ----------------------------------------------------------------------------

#[tokio::test]
async fn host_egress_maps_runtime_credential_error_to_policy_denied() {
    let runtime = Arc::new(RecordingRuntimeEgress::with_error(
        RuntimeHttpEgressError::Credential {
            reason: "credential store unavailable".to_string(),
        },
    ));
    let shim = build_shim(Arc::clone(&runtime), telegram_target_with_default_handle());

    let error = shim
        .send(telegram_send_message_request())
        .await
        .expect_err("runtime credential failure should not be swallowed");

    // Must be PolicyDenied (permanent), NOT UnknownCredentialHandle —
    // stuffing the host's free-text reason into a `handle` slot was the
    // previous bug, and UnknownCredentialHandle being permanent meant a
    // transient `StoreUnavailable` was dropped instead of retried.
    match &error {
        ProtocolHttpEgressError::PolicyDenied { reason } => {
            let rendered = reason.to_string();
            // RedactedString renders as `<redacted>`; we lock in the variant
            // (which drives `is_retryable`), not the reason text.
            assert_eq!(rendered, "<redacted>");
        }
        other => panic!("expected PolicyDenied, got {other:?}"),
    }
    // The adapter retry classifier must agree this is non-retryable.
    let adapter_error: ironclaw_product_adapters::ProductAdapterError = error.into();
    assert!(
        !adapter_error.is_retryable(),
        "credential failures must surface as permanent so duplicate webhooks don't keep retrying"
    );
}

#[tokio::test]
async fn host_egress_maps_runtime_request_error_to_policy_denied() {
    let runtime = Arc::new(RecordingRuntimeEgress::with_error(
        RuntimeHttpEgressError::Request {
            reason: "sensitive_header_denied:authorization".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        },
    ));
    let shim = build_shim(Arc::clone(&runtime), telegram_target_with_default_handle());

    let error = shim
        .send(telegram_send_message_request())
        .await
        .expect_err("request validation failures must surface");

    match &error {
        ProtocolHttpEgressError::PolicyDenied { .. } => {}
        other => panic!("expected PolicyDenied, got {other:?}"),
    }
    let adapter_error: ironclaw_product_adapters::ProductAdapterError = error.into();
    assert!(!adapter_error.is_retryable());
}

#[tokio::test]
async fn host_egress_maps_runtime_network_error_to_retryable_network() {
    let runtime = Arc::new(RecordingRuntimeEgress::with_error(
        RuntimeHttpEgressError::Network {
            reason: "connect timeout".to_string(),
            request_bytes: 32,
            response_bytes: 0,
        },
    ));
    let shim = build_shim(Arc::clone(&runtime), telegram_target_with_default_handle());

    let error = shim
        .send(telegram_send_message_request())
        .await
        .expect_err("network failures must surface");

    match &error {
        ProtocolHttpEgressError::Network(_) => {}
        other => panic!("expected Network, got {other:?}"),
    }
    let adapter_error: ironclaw_product_adapters::ProductAdapterError = error.into();
    assert!(
        adapter_error.is_retryable(),
        "transport-layer failures must surface as retryable so transient connectivity issues recover"
    );
}

#[tokio::test]
async fn host_egress_maps_runtime_response_error_to_permanent_policy_denied() {
    // The pre-fix bug: Response → Network → retryable. The host emits
    // Response for leak-detector matches and decode errors — permanent
    // conditions that retry would never resolve. Mapping to PolicyDenied
    // makes the workflow surface the failure once instead of looping.
    let runtime = Arc::new(RecordingRuntimeEgress::with_error(
        RuntimeHttpEgressError::Response {
            reason: "response_leak_blocked".to_string(),
            request_bytes: 32,
            response_bytes: 0,
        },
    ));
    let shim = build_shim(Arc::clone(&runtime), telegram_target_with_default_handle());

    let error = shim
        .send(telegram_send_message_request())
        .await
        .expect_err("response leak should not be swallowed");

    match &error {
        ProtocolHttpEgressError::PolicyDenied { .. } => {}
        other => panic!("expected PolicyDenied for response-side failures, got {other:?}"),
    }
    let adapter_error: ironclaw_product_adapters::ProductAdapterError = error.into();
    assert!(
        !adapter_error.is_retryable(),
        "leak-detector matches and response-decode errors are permanent; retrying never helps"
    );
}
