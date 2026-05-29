use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer;
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource};
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::{
    AgentId, CapabilityDescriptor, CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet,
    CorrelationId, Decision, EffectKind, ExecutionContext, ExtensionId, GrantConstraints, HostPath,
    InvocationId, MissionId, MountView, NetworkMethod, NetworkPolicy, NetworkScheme,
    NetworkTargetPattern, Obligation, Obligations, PackageId, Principal, ProjectId,
    ResourceEstimate, ResourceScope, RuntimeCredentialTarget, RuntimeKind, SecretHandle, TenantId,
    TrustClass, UserId, VirtualPath,
};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, HostRuntime, HostRuntimeServices, RuntimeCapabilityOutcome,
    RuntimeCapabilityRequest, default_host_api_contract_registry, default_host_port_catalog,
};
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
};
use ironclaw_processes::ProcessServices;
use ironclaw_resources::{
    InMemoryResourceGovernor, ResourceAccount, ResourceGovernor, ResourceLimits,
};
use ironclaw_secrets::{InMemorySecretStore, SecretMaterial, SecretStore};
use ironclaw_trust::{
    AdminConfig, AdminEntry, AuthorityCeiling, EffectiveTrustClass, HostTrustAssignment,
    HostTrustPolicy, TrustDecision, TrustProvenance,
};
use ironclaw_wasm::{
    RecordingWasmHostHttp, WasmHostError, WasmHttpResponse, WasmStagedRuntimeCredential,
    WasmStagedRuntimeCredentials, WitToolExecution, WitToolHost, WitToolRequest, WitToolRuntime,
    WitToolRuntimeConfig,
};
use serde_json::json;

#[tokio::test]
async fn host_runtime_services_routes_github_wasm_read_through_runtime_http_egress() {
    let capability_id = CapabilityId::new("github.search_issues").unwrap();
    let scope = sample_scope(InvocationId::new());
    let expected_url =
        "https://api.github.com/search/issues?q=repo%3Anearai%2Fironclaw%20is%3Aissue&per_page=1";
    let policy = github_policy();
    let network = RecordingNetworkHttpEgress::with_body(
        br#"{"total_count":0,"incomplete_results":false,"items":[]}"#.to_vec(),
    );
    let secret_store = Arc::new(InMemorySecretStore::new());
    let secret_handle = SecretHandle::new("github_token").unwrap();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_github_package()),
        Arc::new(filesystem_with_github_package()),
        Arc::new(governor_with_default_limit(sample_account())),
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ApplyNetworkPolicy {
                policy: policy.clone(),
            },
            Obligation::InjectSecretOnce {
                handle: secret_handle.clone(),
            },
        ])),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_secret_store(Arc::clone(&secret_store))
    .with_trust_policy(Arc::new(github_first_party_trust_policy()))
    .with_wasm_runtime_credential_provider(Arc::new(WasmStagedRuntimeCredentials::new(vec![
        WasmStagedRuntimeCredential::for_exact_url(
            secret_handle.clone(),
            RuntimeCredentialTarget::Header {
                name: "authorization".to_string(),
                prefix: Some("Bearer ".to_string()),
            },
            true,
            expected_url.to_string(),
        ),
    ])))
    .try_with_host_http_egress(network.clone())
    .unwrap()
    .try_with_wasm_runtime(WitToolRuntimeConfig::default(), WitToolHost::deny_all())
    .unwrap();
    secret_store
        .put(
            scope.clone(),
            secret_handle,
            SecretMaterial::from("ghp_fake_fixture_token"),
        )
        .await
        .unwrap();

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"query": "repo:nearai/ironclaw is:issue", "limit": 1}),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id);
            assert_eq!(
                completed.output,
                json!({"total_count":0,"incomplete_results":false,"items":[]})
            );
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
    let requests = network.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, NetworkMethod::Get);
    assert_eq!(requests[0].url, expected_url);
    assert_eq!(requests[0].body, Vec::<u8>::new());
    assert_eq!(requests[0].policy, policy);
    assert_eq!(
        requests[0]
            .headers
            .iter()
            .find(|(name, _)| name == "authorization"),
        Some(&(
            "authorization".to_string(),
            "Bearer ghp_fake_fixture_token".to_string(),
        ))
    );
}

#[tokio::test]
async fn host_runtime_services_missing_github_runtime_secret_blocks_on_auth() {
    let capability_id = CapabilityId::new("github.search_issues").unwrap();
    let scope = sample_scope(InvocationId::new());
    let expected_url =
        "https://api.github.com/search/issues?q=repo%3Anearai%2Fironclaw%20is%3Aissue&per_page=1";
    let network = RecordingNetworkHttpEgress::with_body(
        br#"{"total_count":0,"incomplete_results":false,"items":[]}"#.to_vec(),
    );
    let secret_store = Arc::new(InMemorySecretStore::new());
    let secret_handle = SecretHandle::new("github_token").unwrap();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_github_package()),
        Arc::new(filesystem_with_github_package()),
        Arc::new(governor_with_default_limit(sample_account())),
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ApplyNetworkPolicy {
                policy: github_policy(),
            },
            Obligation::InjectSecretOnce {
                handle: secret_handle.clone(),
            },
        ])),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_secret_store(Arc::clone(&secret_store))
    .with_trust_policy(Arc::new(github_first_party_trust_policy()))
    .with_wasm_runtime_credential_provider(Arc::new(WasmStagedRuntimeCredentials::new(vec![
        WasmStagedRuntimeCredential::for_exact_url(
            secret_handle.clone(),
            RuntimeCredentialTarget::Header {
                name: "authorization".to_string(),
                prefix: Some("Bearer ".to_string()),
            },
            true,
            expected_url.to_string(),
        ),
    ])))
    .try_with_host_http_egress(network.clone())
    .unwrap()
    .try_with_wasm_runtime(WitToolRuntimeConfig::default(), WitToolHost::deny_all())
    .unwrap();

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"query": "repo:nearai/ironclaw is:issue", "limit": 1}),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::AuthRequired(gate) => {
            assert_eq!(gate.capability_id, capability_id);
            assert!(
                gate.required_secrets.is_empty(),
                "secret handles are not product-visible until auth recovery projections carry them"
            );
        }
        other => panic!("expected auth-required outcome, got {other:?}"),
    }
    assert!(
        network.requests().is_empty(),
        "missing credential must block before dispatch"
    );
}

#[tokio::test]
async fn bundled_github_wasm_executes_search_get_and_comment_operations() {
    let search_http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 200,
        headers_json: "{}".to_string(),
        body: br#"{"total_count":0,"incomplete_results":false,"items":[]}"#.to_vec(),
    }));
    let search = execute_bundled_github_wasm(
        "github.search_issues",
        json!({"query": "repo:nearai/ironclaw is:issue", "limit": 1}),
        Arc::clone(&search_http),
    );
    assert_eq!(search.error, None);
    assert_eq!(
        search.output_json.as_deref(),
        Some(r#"{"total_count":0,"incomplete_results":false,"items":[]}"#)
    );
    assert_single_wasm_request(
        &search_http,
        "GET",
        "https://api.github.com/search/issues?q=repo%3Anearai%2Fironclaw%20is%3Aissue&per_page=1",
        None,
    );

    let get_issue_http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 200,
        headers_json: "{}".to_string(),
        body: br#"{"number":2,"title":"Reborn GitHub issue","state":"open","html_url":"https://github.com/nearai/ironclaw/issues/2"}"#.to_vec(),
    }));
    let get_issue = execute_bundled_github_wasm(
        "github.get_issue",
        json!({"owner": "nearai", "repo": "ironclaw", "issue_number": 2}),
        Arc::clone(&get_issue_http),
    );
    assert_eq!(get_issue.error, None);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(get_issue.output_json.as_deref().unwrap())
            .unwrap()["number"],
        json!(2)
    );
    assert_single_wasm_request(
        &get_issue_http,
        "GET",
        "https://api.github.com/repos/nearai/ironclaw/issues/2",
        None,
    );

    let comment_http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 201,
        headers_json: "{}".to_string(),
        body: br##"{"id":44,"html_url":"https://github.com/nearai/ironclaw/issues/2#issuecomment-44","body":"Reborn WASM comment"}"##.to_vec(),
    }));
    let comment = execute_bundled_github_wasm(
        "github.comment_issue",
        json!({
            "owner": "nearai",
            "repo": "ironclaw",
            "issue_number": 2,
            "body": "Reborn WASM comment",
        }),
        Arc::clone(&comment_http),
    );
    assert_eq!(comment.error, None);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(comment.output_json.as_deref().unwrap()).unwrap()
            ["body"],
        json!("Reborn WASM comment")
    );
    assert_single_wasm_request(
        &comment_http,
        "POST",
        "https://api.github.com/repos/nearai/ironclaw/issues/2/comments",
        Some(br#"{"body":"Reborn WASM comment"}"#),
    );
}

#[tokio::test]
async fn bundled_github_wasm_sanitizes_host_http_and_api_failures() {
    let cases = [
        (
            RecordingWasmHostHttp::err(WasmHostError::Unavailable(
                "missing auth token ghp_fake_fixture_token".to_string(),
            )),
            "AuthRequired",
        ),
        (
            RecordingWasmHostHttp::err(WasmHostError::Failed(
                "deadline exceeded while token ghp_fake_fixture_token was present".to_string(),
            )),
            "AuthRequired",
        ),
        (
            RecordingWasmHostHttp::err(WasmHostError::Failed("redirect blocked".to_string())),
            "github_api_redirect_denied",
        ),
        (
            RecordingWasmHostHttp::err(WasmHostError::FailedAfterRequestSent(
                "response body too large".to_string(),
            )),
            "github_api_body_limit",
        ),
        (
            RecordingWasmHostHttp::err(WasmHostError::Denied(
                "host not allowed: api.evil.test".to_string(),
            )),
            "github_api_egress_denied",
        ),
        (
            RecordingWasmHostHttp::ok(WasmHttpResponse {
                status: 403,
                headers_json: "{}".to_string(),
                body: br#"{"message":"bad credentials ghp_fake_fixture_token"}"#.to_vec(),
            }),
            "github_api_error_status_403",
        ),
        (
            RecordingWasmHostHttp::ok(WasmHttpResponse {
                status: 200,
                headers_json: "{}".to_string(),
                body: vec![0xff, 0xfe],
            }),
            "github_api_invalid_utf8",
        ),
    ];

    for (http, expected_error) in cases {
        let execution = execute_bundled_github_wasm(
            "github.search_issues",
            json!({"query": "repo:nearai/ironclaw is:issue", "limit": 1}),
            Arc::new(http),
        );
        assert_eq!(execution.error.as_deref(), Some(expected_error));
        assert!(
            !format!("{execution:?}").contains("ghp_fake_fixture_token"),
            "guest-visible failure must not leak credential material"
        );
    }
}

#[tokio::test]
async fn bundled_github_wasm_leaves_success_json_for_host_output_decode() {
    let execution = execute_bundled_github_wasm(
        "github.search_issues",
        json!({"query": "repo:nearai/ironclaw is:issue", "limit": 1}),
        Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
            status: 200,
            headers_json: "{}".to_string(),
            body: b"not-json".to_vec(),
        })),
    );

    assert_eq!(execution.output_json.as_deref(), Some("not-json"));
    assert_eq!(execution.error, None);
}

#[derive(Debug, Clone)]
struct RecordingNetworkHttpEgress {
    requests: Arc<std::sync::Mutex<Vec<NetworkHttpRequest>>>,
    response_body: Vec<u8>,
}

impl RecordingNetworkHttpEgress {
    fn with_body(response_body: Vec<u8>) -> Self {
        Self {
            requests: Arc::new(std::sync::Mutex::new(Vec::new())),
            response_body,
        }
    }

    fn requests(&self) -> Vec<NetworkHttpRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl NetworkHttpEgress for RecordingNetworkHttpEgress {
    async fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let request_bytes = request.body.len() as u64;
        self.requests.lock().unwrap().push(request);
        Ok(NetworkHttpResponse {
            status: 200,
            headers: Vec::new(),
            body: self.response_body.clone(),
            usage: NetworkUsage {
                request_bytes,
                response_bytes: self.response_body.len() as u64,
                resolved_ip: None,
            },
        })
    }
}

struct ObligatingAuthorizer {
    obligations: Vec<Obligation>,
}

impl ObligatingAuthorizer {
    fn new(obligations: Vec<Obligation>) -> Self {
        Self { obligations }
    }
}

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for ObligatingAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::new(self.obligations.clone()).unwrap(),
        }
    }

    async fn authorize_spawn_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: Obligations::new(self.obligations.clone()).unwrap(),
        }
    }
}

fn registry_with_github_package() -> ExtensionRegistry {
    let manifest = ExtensionManifest::parse_with_host_api_contracts(
        &std::fs::read_to_string(github_asset_root().join("manifest.toml")).unwrap(),
        ManifestSource::HostBundled,
        &default_host_port_catalog().unwrap(),
        &default_host_api_contract_registry().unwrap(),
    )
    .unwrap();
    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new("/system/extensions/github").unwrap(),
    )
    .unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
}

fn filesystem_with_github_package() -> LocalFilesystem {
    let mut filesystem = LocalFilesystem::new();
    filesystem
        .mount_local(
            VirtualPath::new("/system/extensions").unwrap(),
            HostPath::from_path_buf(github_asset_root().parent().unwrap().to_path_buf()),
        )
        .unwrap();
    filesystem
}

fn github_asset_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("crates/ironclaw_first_party_extensions/assets/github")
}

fn github_wasm_path() -> std::path::PathBuf {
    github_asset_root().join("wasm/github_tool.wasm")
}

fn github_first_party_trust_policy() -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("github").unwrap(),
            "/system/extensions/github/manifest.toml".to_string(),
            None,
            HostTrustAssignment::first_party(),
            vec![
                EffectKind::DispatchCapability,
                EffectKind::Network,
                EffectKind::UseSecret,
                EffectKind::ExternalWrite,
            ],
            None,
        ),
    ]))])
    .unwrap()
}

fn wasm_runtime_request_for_scope(
    capability_id: CapabilityId,
    scope: ResourceScope,
    input: serde_json::Value,
) -> RuntimeCapabilityRequest {
    let context = execution_context_with_dispatch_grant_for_scope(capability_id.clone(), scope);
    RuntimeCapabilityRequest::new(
        context,
        capability_id,
        wasm_http_estimate(),
        input,
        trust_decision_with_dispatch_authority(),
    )
}

fn execution_context_with_dispatch_grant_for_scope(
    capability: CapabilityId,
    scope: ResourceScope,
) -> ExecutionContext {
    let context = ExecutionContext {
        invocation_id: scope.invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        mission_id: scope.mission_id.clone(),
        thread_id: scope.thread_id.clone(),
        extension_id: ExtensionId::new("caller").unwrap(),
        runtime: RuntimeKind::Wasm,
        trust: TrustClass::UserTrusted,
        grants: capability_grants(capability),
        mounts: MountView::default(),
        resource_scope: scope,
    };
    context.validate().unwrap();
    context
}

fn capability_grants(capability: CapabilityId) -> CapabilitySet {
    let mut grants = CapabilitySet::default();
    grants.grants.push(CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability,
        grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
        issued_by: Principal::HostRuntime,
        constraints: GrantConstraints {
            allowed_effects: vec![
                EffectKind::DispatchCapability,
                EffectKind::Network,
                EffectKind::UseSecret,
                EffectKind::ExternalWrite,
            ],
            mounts: MountView::default(),
            network: NetworkPolicy::default(),
            secrets: vec![SecretHandle::new("github_token").unwrap()],
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    });
    grants
}

fn trust_decision_with_dispatch_authority() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: vec![
                EffectKind::DispatchCapability,
                EffectKind::Network,
                EffectKind::UseSecret,
                EffectKind::ExternalWrite,
            ],
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: Utc::now(),
    }
}

fn execute_bundled_github_wasm(
    capability_id: &str,
    input: serde_json::Value,
    http: Arc<RecordingWasmHostHttp>,
) -> WitToolExecution {
    let runtime = WitToolRuntime::new(WitToolRuntimeConfig::default()).unwrap();
    let wasm_bytes =
        std::fs::read(github_wasm_path()).expect("first-party GitHub WASM must be built");
    let prepared = runtime.prepare("github", &wasm_bytes).unwrap();
    runtime
        .execute(
            &prepared,
            WitToolHost::deny_all().with_http(http),
            WitToolRequest::new(input.to_string()).with_context(
                json!({
                    "capability_id": capability_id,
                })
                .to_string(),
            ),
        )
        .unwrap()
}

fn assert_single_wasm_request(
    http: &RecordingWasmHostHttp,
    expected_method: &str,
    expected_url: &str,
    expected_body: Option<&[u8]>,
) {
    let requests = http.requests().unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, expected_method);
    assert_eq!(request.url, expected_url);
    assert_eq!(request.timeout_ms, Some(10_000));
    assert_eq!(request.body.as_deref(), expected_body);

    let headers: serde_json::Value = serde_json::from_str(&request.headers_json).unwrap();
    assert_eq!(headers["User-Agent"], "IronClaw-GitHub-Reborn-WASM");
    assert_eq!(headers["X-GitHub-Api-Version"], "2026-03-10");
}

fn governor_with_default_limit(account: ResourceAccount) -> InMemoryResourceGovernor {
    let governor = InMemoryResourceGovernor::new();
    governor
        .set_limit(
            account,
            ResourceLimits {
                max_concurrency_slots: Some(10),
                max_network_egress_bytes: Some(10_000),
                max_output_bytes: Some(100_000),
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    governor
}

fn wasm_http_estimate() -> ResourceEstimate {
    ResourceEstimate {
        concurrency_slots: Some(1),
        network_egress_bytes: Some(10),
        output_bytes: Some(10_000),
        ..ResourceEstimate::default()
    }
}

fn sample_account() -> ResourceAccount {
    ResourceAccount::tenant(TenantId::new("tenant-a").unwrap())
}

fn sample_scope(invocation_id: InvocationId) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").unwrap(),
        user_id: UserId::new("user-a").unwrap(),
        agent_id: Some(AgentId::new("agent-a").unwrap()),
        project_id: Some(ProjectId::new("project-a").unwrap()),
        mission_id: Some(MissionId::new("mission-a").unwrap()),
        thread_id: None,
        invocation_id,
    }
}

fn github_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.github.com".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(10_000),
    }
}
