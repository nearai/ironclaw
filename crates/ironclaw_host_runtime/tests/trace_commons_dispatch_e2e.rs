//! End-to-end test: agent invokes `builtin.trace_commons.onboard` and
//! `builtin.trace_commons.status` through the host-runtime capability-dispatch
//! path (not directly through the library).
//!
//! This file is a separate test binary so it gets a fresh process — and
//! therefore a fresh `LazyLock` for `ironclaw_base_dir()`. `IRONCLAW_BASE_DIR`
//! is set to a tempdir as the very first action so the LazyLock picks it up.
//!
//! Both tests share the tempdir (base dir is process-wide); they use different
//! mock ports / invite codes / user scopes to avoid cross-test interference.

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use axum::{Router, extract::State, routing::post};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::{
    runtime_policy::{
        ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
        NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
    },
    *,
};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, HostRuntime, HostRuntimeServices, RuntimeCapabilityOutcome,
    RuntimeCapabilityRequest, RuntimeFailureKind, TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
    TRACE_COMMONS_STATUS_CAPABILITY_ID, builtin_first_party_handlers, builtin_first_party_package,
};
use ironclaw_network::{PolicyNetworkHttpEgress, ReqwestNetworkTransport};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_secrets::InMemorySecretStore;
use ironclaw_triggers::InMemoryTriggerRepository;
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

// ── Process-wide base-dir setup ──────────────────────────────────────────────

static BASE_DIR: std::sync::OnceLock<TempDir> = std::sync::OnceLock::new();

/// Point `IRONCLAW_BASE_DIR` at a process-lifetime temp dir before any code
/// reads it. `#[tokio::test]` uses a multi-threaded runtime and tests in this
/// binary run concurrently, so this must be the FIRST call in every test:
/// the `OnceLock` serializes initialization (concurrent callers block until
/// the variable is set) and no test reads `ironclaw_base_dir()` before its
/// own `setup_base_dir()` call returns.
fn setup_base_dir() -> &'static TempDir {
    BASE_DIR.get_or_init(|| {
        let dir = tempfile::tempdir().expect("tempdir for IRONCLAW_BASE_DIR");
        // SAFETY: executed exactly once inside `OnceLock::get_or_init`; every
        // test calls `setup_base_dir()` before any env read, so there is no
        // concurrent reader during this write.
        unsafe {
            std::env::set_var("IRONCLAW_BASE_DIR", dir.path());
        }
        dir
    })
}

// ── Mock issuer helpers ──────────────────────────────────────────────────────

/// Axum handler state: (canned response JSON, status code, received bodies).
type MockState = Arc<(
    serde_json::Value,
    axum::http::StatusCode,
    Arc<Mutex<Vec<serde_json::Value>>>,
)>;

/// Derive `sha256:<lowercase_hex>` from a base64-STANDARD-encoded public key —
/// mirrors `device_key::device_key_id_from_pubkey` on the server side.
fn derive_device_key_id(pubkey_b64: &str) -> Option<String> {
    let bytes = BASE64_STANDARD.decode(pubkey_b64).ok()?;
    Some(format!("sha256:{}", hex::encode(Sha256::digest(&bytes))))
}

/// Sentinel: the mock replaces this with the derived key id for the request's
/// submitted public key, so the client's cross-check always passes.
const ECHO_DEVICE_KEY_ID: &str = "ECHO_DEVICE_KEY_ID";

/// Spawn a mock `/v1/onboard` axum server on `127.0.0.1:0`.
/// `make_response` receives the bound address so it can embed the correct
/// `issuer_url` in the JSON.
async fn spawn_mock_issuer<F>(
    make_response: F,
    status: axum::http::StatusCode,
) -> (SocketAddr, Arc<Mutex<Vec<serde_json::Value>>>)
where
    F: Fn(SocketAddr) -> serde_json::Value + Send + Sync + 'static,
{
    let received: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock issuer binds");
    let addr = listener.local_addr().expect("mock issuer local addr");

    let response_body = make_response(addr);
    let state: MockState = Arc::new((response_body, status, Arc::clone(&received)));

    async fn handler(
        State(state): State<MockState>,
        axum::Json(body): axum::Json<serde_json::Value>,
    ) -> axum::response::Response {
        let mut response = state.0.clone();
        // Echo the correct device_key_id if the canned response uses the sentinel.
        if response.get("device_key_id").and_then(|v| v.as_str()) == Some(ECHO_DEVICE_KEY_ID)
            && let Some(pubkey_b64) = body.get("device_public_key").and_then(|v| v.as_str())
            && let Some(derived) = derive_device_key_id(pubkey_b64)
        {
            response["device_key_id"] = serde_json::Value::String(derived);
        }
        state.2.lock().unwrap().push(body);
        let json_bytes = serde_json::to_vec(&response).unwrap();
        axum::response::Response::builder()
            .status(state.1)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(json_bytes))
            .unwrap()
    }

    let app = Router::new()
        .route("/v1/onboard", post(handler))
        .with_state(state);

    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (addr, received)
}

// ── Runtime / dispatch helpers ───────────────────────────────────────────────

fn registry() -> ExtensionRegistry {
    let mut r = ExtensionRegistry::new();
    r.insert(builtin_first_party_package().unwrap()).unwrap();
    r
}

fn builtin_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::ReadFilesystem,
        EffectKind::WriteFilesystem,
        EffectKind::Network,
        EffectKind::SpawnProcess,
        EffectKind::ExecuteCode,
        EffectKind::ExternalWrite,
    ]
}

fn trust_policy() -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("builtin").unwrap(),
            "/system/extensions/builtin/manifest.toml".to_string(),
            None,
            HostTrustAssignment::first_party(),
            builtin_effects(),
            None,
        ),
    ]))])
    .unwrap()
}

fn trust_decision() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: builtin_effects(),
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: chrono::Utc::now(),
    }
}

/// A runtime policy that permits outbound network calls (DirectLogged = no
/// deny; allows loopback HTTP that onboarding uses).
fn network_permitted_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalDev,
        resolved_profile: RuntimeProfile::LocalDev,
        filesystem_backend: FilesystemBackendKind::HostWorkspace,
        process_backend: ProcessBackendKind::LocalHost,
        network_mode: NetworkMode::DirectLogged,
        secret_mode: SecretMode::ScrubbedEnv,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::LocalMinimal,
    }
}

fn capability_id(value: &str) -> CapabilityId {
    CapabilityId::new(value).unwrap()
}

fn dispatch_grant_with_network(capability: &str, network: NetworkPolicy) -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id(capability),
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: builtin_effects(),
            mounts: MountView::default(),
            network,
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

/// `execution_context` granting the given capability with network allowed.
fn execution_context_with_network(capability: &str, network: NetworkPolicy) -> ExecutionContext {
    let capability_set = CapabilitySet {
        grants: vec![dispatch_grant_with_network(capability, network)],
    };
    ExecutionContext::local_default(
        UserId::new("user").unwrap(),
        ExtensionId::new("caller").unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::FirstParty,
        capability_set,
        MountView::default(),
    )
    .unwrap()
}

/// Execution context granting a read-only capability (no network needed).
fn execution_context_read_only(capability: &str) -> ExecutionContext {
    execution_context_with_network(capability, NetworkPolicy::default())
}

/// Network policy that allows loopback HTTP to the mock issuer.
///
/// The real `PolicyNetworkHttpEgress` does exact-host matching (a bare `"*"`
/// host pattern does NOT wildcard-match), so we allowlist `127.0.0.1`
/// explicitly and opt out of the private-IP block so the agent onboard POST can
/// reach the loopback mock.
fn allow_all_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: None,
            host_pattern: "127.0.0.1".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: false,
        max_egress_bytes: None,
    }
}

/// Production-default network policy: private/loopback IP ranges are denied.
/// Allowlists `127.0.0.1` by host pattern, but the private-IP block must still
/// reject the loopback destination — demonstrating #4560: the agent cannot
/// reach private destinations through onboarding once the policy is enforced.
fn deny_private_ip_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: None,
            host_pattern: "127.0.0.1".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: None,
    }
}

fn runtime() -> impl HostRuntime {
    // Use a real PolicyNetworkHttpEgress backed by ReqwestNetworkTransport so the
    // InvocationServices builder is satisfied (it requires an egress when the
    // capability declares EffectKind::Network). The trace_commons.onboard handler
    // makes its own reqwest calls internally; the egress here is only needed to
    // pass the service-builder check.
    let network = PolicyNetworkHttpEgress::new(ReqwestNetworkTransport::new(
        std::time::Duration::from_secs(30),
    ));
    HostRuntimeServices::new(
        Arc::new(registry()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(
        builtin_first_party_handlers(Arc::new(InMemoryTriggerRepository::default())).unwrap(),
    ))
    .with_secret_store(Arc::new(InMemorySecretStore::new()))
    .try_with_host_http_egress(network)
    .expect("real http egress wiring must succeed")
    .with_runtime_policy(network_permitted_policy())
    .with_trust_policy(Arc::new(trust_policy()))
    .host_runtime_for_local_testing()
}

async fn invoke_with_context(
    runtime: &impl HostRuntime,
    capability: &str,
    input: Value,
    context: ExecutionContext,
) -> Result<Value, RuntimeFailureKind> {
    let outcome = runtime
        .invoke_capability(RuntimeCapabilityRequest::new(
            context,
            capability_id(capability),
            ResourceEstimate::default(),
            input,
            trust_decision(),
        ))
        .await
        .unwrap();
    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => Ok(completed.output),
        RuntimeCapabilityOutcome::Failed(failure) => Err(failure.kind),
        other => panic!("unexpected capability outcome: {other:?}"),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Verify the full dispatch chain:
///   agent → `builtin.trace_commons.onboard` (confirmed=true)
///         → host runtime dispatch
///         → `ironclaw_reborn_traces::onboarding::onboard()`
///         → real HTTP POST to mock issuer
///         → policy written
///   then `builtin.trace_commons.status` reports enrolled.
#[tokio::test]
async fn onboard_then_status_through_dispatch() {
    let _base_dir = setup_base_dir();

    let (addr, received) = spawn_mock_issuer(
        |addr| {
            json!({
                "schema_version": "trace_commons.onboard_response.v1",
                "tenant_id": "tenant-e2e",
                "ingest_url": "https://ingest.example.com",
                "issuer_url": format!("http://127.0.0.1:{}", addr.port()),
                "audience": "trace-commons-ingest",
                "device_key_id": ECHO_DEVICE_KEY_ID,
                "profile_url": "https://tracecommons.ai/profile",
                "leaderboard_url": "https://tracecommons.ai/lb",
            })
        },
        axum::http::StatusCode::OK,
    )
    .await;

    let invite_url = format!("http://127.0.0.1:{}/onboard#INVE2E001", addr.port());
    let rt = runtime();

    // ── invoke onboard ────────────────────────────────────────────────────────
    let onboard_result = invoke_with_context(
        &rt,
        TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
        json!({
            "invite_url": invite_url,
            "include_message_text": true,
            "include_tool_payloads": false,
            "confirmed": true,
        }),
        execution_context_with_network(
            TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
            allow_all_network_policy(),
        ),
    )
    .await
    .expect("onboard dispatch must succeed (Ok variant)");

    // ── assert onboard response fields ────────────────────────────────────────
    assert_eq!(
        onboard_result["enrolled"],
        json!(true),
        "enrolled must be true after successful onboard"
    );
    assert_eq!(
        onboard_result["tenant_id"],
        json!("tenant-e2e"),
        "tenant_id must match mock response"
    );
    let device_key_id = onboard_result["device_key_id"]
        .as_str()
        .expect("device_key_id must be a string");
    assert!(
        device_key_id.starts_with("sha256:"),
        "device_key_id must start with 'sha256:'"
    );
    assert_eq!(
        onboard_result["consents"]["include_message_text"],
        json!(true),
        "include_message_text consent must be recorded"
    );
    assert_eq!(
        onboard_result["consents"]["include_tool_payloads"],
        json!(false),
        "include_tool_payloads consent must be recorded"
    );

    // ── assert mock received exactly 1 request with expected fields ───────────
    {
        let requests = received.lock().unwrap();
        assert_eq!(requests.len(), 1, "mock must receive exactly 1 POST");
        let req = &requests[0];
        let pubkey = req["device_public_key"]
            .as_str()
            .expect("device_public_key must be present");
        assert!(!pubkey.is_empty(), "device_public_key must not be empty");
        assert_eq!(
            req["invite_code"],
            json!("INVE2E001"),
            "invite_code must match the fragment in the invite URL"
        );
    }

    // ── invoke status ─────────────────────────────────────────────────────────
    let status_result = invoke_with_context(
        &rt,
        TRACE_COMMONS_STATUS_CAPABILITY_ID,
        json!({}),
        execution_context_read_only(TRACE_COMMONS_STATUS_CAPABILITY_ID),
    )
    .await
    .expect("status dispatch must succeed");

    assert_eq!(
        status_result["enrolled"],
        json!(true),
        "status must report enrolled after onboard"
    );
    assert_eq!(
        status_result["tenant_id"],
        json!("tenant-e2e"),
        "status tenant_id must match"
    );
    assert_eq!(
        status_result["auth_mode"],
        json!("device_key"),
        "auth_mode must be device_key"
    );
    assert_eq!(
        status_result["include_message_text"],
        json!(true),
        "status include_message_text must reflect consents"
    );
}

/// #4560: with the production-default network policy (private/loopback IP
/// ranges denied), the agent onboard POST to a 127.0.0.1 invite must be blocked
/// by the host network-egress policy — the tool reports a network failure and
/// does NOT enroll. This is the regression test demonstrating the fix: the agent
/// can no longer reach private destinations through onboarding.
#[tokio::test]
async fn onboard_private_ip_blocked_by_network_policy() {
    let _base_dir = setup_base_dir();

    let (addr, received) = spawn_mock_issuer(
        |addr| {
            json!({
                "schema_version": "trace_commons.onboard_response.v1",
                "tenant_id": "tenant-blocked",
                "ingest_url": "https://ingest.example.com",
                "issuer_url": format!("http://127.0.0.1:{}", addr.port()),
                "audience": "trace-commons-ingest",
                "device_key_id": ECHO_DEVICE_KEY_ID,
            })
        },
        axum::http::StatusCode::OK,
    )
    .await;

    let invite_url = format!("http://127.0.0.1:{}/onboard#INVE2E003", addr.port());
    let rt = runtime();

    let result = invoke_with_context(
        &rt,
        TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
        json!({
            "invite_url": invite_url,
            "include_message_text": false,
            "include_tool_payloads": false,
            "confirmed": true,
        }),
        execution_context_with_network(
            TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
            deny_private_ip_network_policy(),
        ),
    )
    .await
    .expect("onboard dispatch returns an Ok envelope even when egress is blocked");

    assert_ne!(
        result["enrolled"],
        json!(true),
        "onboard must NOT enroll when the network policy blocks the private-IP destination"
    );
    assert_eq!(
        result["error_code"],
        json!("Network"),
        "blocked egress must surface as a network failure (invite not consumed)"
    );

    // The policy blocks before any bytes reach the wire: the mock must see no
    // request body parsed (the egress denies the private-IP target pre-flight).
    let requests = received.lock().unwrap();
    assert_eq!(
        requests.len(),
        0,
        "no onboarding POST may reach the private-IP destination once the policy denies it"
    );
}

/// Verify that `confirmed=false` short-circuits before making ANY network call.
#[tokio::test]
async fn onboard_unconfirmed_makes_no_network_call() {
    let _base_dir = setup_base_dir();

    let (addr, received) = spawn_mock_issuer(
        |addr| {
            json!({
                "schema_version": "trace_commons.onboard_response.v1",
                "tenant_id": "tenant-unconfirmed",
                "ingest_url": "https://ingest.example.com",
                "issuer_url": format!("http://127.0.0.1:{}", addr.port()),
                "audience": "trace-commons-ingest",
                "device_key_id": ECHO_DEVICE_KEY_ID,
            })
        },
        axum::http::StatusCode::OK,
    )
    .await;

    let invite_url = format!("http://127.0.0.1:{}/onboard#INVE2E002", addr.port());
    let rt = runtime();

    let result = invoke_with_context(
        &rt,
        TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
        json!({
            "invite_url": invite_url,
            "include_message_text": false,
            "include_tool_payloads": false,
            "confirmed": false,
        }),
        execution_context_with_network(
            TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
            allow_all_network_policy(),
        ),
    )
    .await
    .expect("unconfirmed dispatch must succeed (Ok variant with consent gate)");

    // The consent gate returns enrolled=false + consent_required=true.
    assert_eq!(
        result["enrolled"],
        json!(false),
        "enrolled must be false for unconfirmed call"
    );
    assert_eq!(
        result["consent_required"],
        json!(true),
        "consent_required must be true"
    );

    // The mock must have received ZERO requests — no network call made.
    let requests = received.lock().unwrap();
    assert_eq!(
        requests.len(),
        0,
        "no HTTP requests must reach the mock when confirmed=false"
    );
}
