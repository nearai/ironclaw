//! Shared setup for the Trace Commons capability-dispatch e2e binaries.
//!
//! Included by `trace_commons_dispatch_e2e.rs` (personal-invite enrollment) and
//! `trace_commons_instance_dispatch_e2e.rs` (admin-provisioned instance
//! enrollment) via `#[path = "support/trace_commons_dispatch.rs"] mod ...`.
//!
//! Each `tests/*.rs` file compiles to a SEPARATE test binary, so each gets a
//! fresh process — and therefore a fresh `LazyLock` for `ironclaw_base_dir()`
//! plus a private `IRONCLAW_BASE_DIR` tempdir. That process boundary is what
//! keeps the two suites isolated: the instance suite writes the process-global
//! instance policy (scope `None`), which must not bleed into the personal suite
//! whose tests rely on per-user not-enrolled defaults.

// `unreachable_pub`: this module is `#[path]`-included into multiple test
// binaries; each uses a different subset of these `pub` helpers, so the unused
// ones would otherwise warn.
#![allow(dead_code, unreachable_pub)]

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use axum::{Router, extract::State, routing::post};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::DiskFilesystem;
use ironclaw_host_api::{
    runtime_policy::{
        ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
        NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
    },
    *,
};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, HostRuntime, HostRuntimeServices, RuntimeCapabilityOutcome,
    RuntimeCapabilityRequest, RuntimeFailureKind, builtin_first_party_handlers,
    builtin_first_party_package,
};
use ironclaw_network::{PolicyNetworkHttpEgress, ReqwestNetworkTransport};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_secrets::InMemorySecretStore;
use ironclaw_triggers::InMemoryTriggerRepository;
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
use serde_json::Value;
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
pub fn setup_base_dir() -> &'static TempDir {
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
pub fn derive_device_key_id(pubkey_b64: &str) -> Option<String> {
    let bytes = BASE64_STANDARD.decode(pubkey_b64).ok()?;
    Some(format!("sha256:{}", hex::encode(Sha256::digest(&bytes))))
}

/// Sentinel: the mock replaces this with the derived key id for the request's
/// submitted public key, so the client's cross-check always passes.
pub const ECHO_DEVICE_KEY_ID: &str = "ECHO_DEVICE_KEY_ID";

/// Spawn a mock `/v1/onboard` axum server on `127.0.0.1:0`.
/// `make_response` receives the bound address so it can embed the correct
/// `issuer_url` in the JSON.
pub async fn spawn_mock_issuer<F>(
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
        #[allow(clippy::let_underscore_must_use)]
        // Background test server; the serve result is unused for test lifetime.
        let _ = axum::serve(listener, app).await;
    });

    (addr, received)
}

// ── Runtime / dispatch helpers ───────────────────────────────────────────────

pub fn registry() -> ExtensionRegistry {
    let mut r = ExtensionRegistry::new();
    r.insert(builtin_first_party_package().unwrap()).unwrap();
    r
}

pub fn builtin_effects() -> Vec<EffectKind> {
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

pub fn trust_policy() -> HostTrustPolicy {
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

pub fn trust_decision() -> TrustDecision {
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
pub fn network_permitted_policy() -> EffectiveRuntimePolicy {
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

pub fn capability_id(value: &str) -> CapabilityId {
    CapabilityId::new(value).unwrap()
}

pub fn dispatch_grant_with_network(
    caller_extension_id: &str,
    capability: &str,
    network: NetworkPolicy,
) -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability: capability_id(capability),
        grantee: Principal::Extension(ExtensionId::new(caller_extension_id).unwrap()),
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
///
/// The base dir is process-wide across a test binary, so onboarding state is
/// keyed by `user_id`. Each test must pass a distinct `user_id` (and matching
/// `caller_extension_id`) so its enrollment state cannot bleed into another test
/// running concurrently in the same process.
pub fn execution_context_with_network(
    user_id: &str,
    caller_extension_id: &str,
    capability: &str,
    network: NetworkPolicy,
) -> ExecutionContext {
    let capability_set = CapabilitySet {
        grants: vec![dispatch_grant_with_network(
            caller_extension_id,
            capability,
            network,
        )],
    };
    ExecutionContext::local_default(
        UserId::new(user_id).unwrap(),
        ExtensionId::new(caller_extension_id).unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::FirstParty,
        capability_set,
        MountView::default(),
    )
    .unwrap()
}

/// Execution context granting a read-only capability (no network needed).
pub fn execution_context_read_only(
    user_id: &str,
    caller_extension_id: &str,
    capability: &str,
) -> ExecutionContext {
    execution_context_with_network(
        user_id,
        caller_extension_id,
        capability,
        NetworkPolicy::default(),
    )
}

/// Network policy that allows loopback HTTP to the mock issuer.
///
/// The real `PolicyNetworkHttpEgress` does exact-host matching (a bare `"*"`
/// host pattern does NOT wildcard-match), so we allowlist `127.0.0.1`
/// explicitly and opt out of the private-IP block so the agent onboard POST can
/// reach the loopback mock.
pub fn allow_all_network_policy() -> NetworkPolicy {
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
pub fn deny_private_ip_network_policy() -> NetworkPolicy {
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

pub fn runtime() -> impl HostRuntime {
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
        Arc::new(DiskFilesystem::new()),
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

pub async fn invoke_with_context(
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

/// A syntactically valid EdDSA JWT (`header.body.signature`) that satisfies
/// `validate_trace_upload_claim_response` — only the shape is checked in these
/// tests, not the signature.
pub fn test_jwt_eddsa(kid: &str) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let header = serde_json::json!({"alg": "EdDSA", "kid": kid});
    format!(
        "{}.{}.signature",
        URL_SAFE_NO_PAD.encode(header.to_string().as_bytes()),
        URL_SAFE_NO_PAD.encode(b"{}")
    )
}

/// Recursively search `dir` for an `account_login_link.<uuid>.url` delivery
/// file and return its contents. Each mint writes a uniquely-named file (so
/// concurrent mints cannot clobber each other); tests assert the secret URL is
/// delivered there rather than on the model-visible surface.
pub fn find_persisted_login_link(dir: &std::path::Path) -> Option<String> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_persisted_login_link(&path) {
                return Some(found);
            }
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("account_login_link.") && n.ends_with(".url"))
        {
            return std::fs::read_to_string(&path).ok();
        }
    }
    None
}
