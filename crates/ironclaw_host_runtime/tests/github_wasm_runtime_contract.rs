use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer;
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource};
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::{
    AgentId, CapabilityDescriptor, CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet,
    CorrelationId, CredentialStageError, Decision, EffectKind, ExecutionContext, ExtensionId,
    GrantConstraints, HostPath, InvocationId, MissionId, MountView, NetworkMethod, NetworkPolicy,
    NetworkScheme, NetworkTargetPattern, Obligation, Obligations, PackageId, Principal, ProjectId,
    ResourceEstimate, ResourceScope, RuntimeCredentialAccountProviderId, RuntimeKind, SecretHandle,
    TenantId, TrustClass, UserId, VirtualPath,
};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, HostRuntime, HostRuntimeServices, RuntimeCapabilityOutcome,
    RuntimeCapabilityRequest, RuntimeCredentialAccessSecret, RuntimeCredentialAccountRequest,
    RuntimeCredentialAccountResolver, RuntimeFailureKind, default_host_api_contract_registry,
    default_host_port_catalog,
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
    RecordingWasmHostHttp, WasmHostError, WasmHttpResponse, WitToolExecution, WitToolHost,
    WitToolRequest, WitToolRuntime, WitToolRuntimeConfig,
};
use serde_json::json;

macro_rules! google_wasm_services_for_test {
    (
        $package_id:expr,
        $policy:expr,
        $network:expr,
        $secret_store:expr,
        $account_access_secret:expr,
        $required_scopes:expr $(,)?
    ) => {{
        let package_id = $package_id;
        let policy = $policy;
        let required_scopes = $required_scopes;
        HostRuntimeServices::new(
            Arc::new(registry_with_google_package(package_id)),
            Arc::new(filesystem_with_google_package(package_id)),
            Arc::new(governor_with_default_limit(sample_account())),
            Arc::new(ObligatingAuthorizer::new(vec![
                Obligation::ApplyNetworkPolicy {
                    policy: policy.clone(),
                },
                Obligation::InjectCredentialAccountOnce {
                    handle: SecretHandle::new("google_runtime_token").unwrap(),
                    provider: RuntimeCredentialAccountProviderId::new("google").unwrap(),
                    setup: ironclaw_host_api::RuntimeCredentialAccountSetup::OAuth {
                        scopes: required_scopes.clone(),
                    },
                    provider_scopes: required_scopes.clone(),
                    requester_extension: ExtensionId::new(package_id).unwrap(),
                },
            ])),
            ProcessServices::in_memory(),
            CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        )
        .with_secret_store($secret_store)
        .with_runtime_credential_account_resolver(Arc::new(
            FixedGoogleRuntimeCredentialAccountResolver {
                expected_requester_extension: ExtensionId::new(package_id).unwrap(),
                expected_scopes: required_scopes,
                result: Ok($account_access_secret),
            },
        ))
        .with_trust_policy(Arc::new(google_first_party_trust_policy(package_id)))
        .try_with_host_http_egress($network)
        .unwrap()
        .try_with_wasm_runtime(WitToolRuntimeConfig::default(), WitToolHost::deny_all())
        .unwrap()
    }};
}

#[tokio::test]
async fn host_runtime_services_routes_structured_github_wasm_search_through_runtime_http_egress() {
    let capability_id = CapabilityId::new("github.search_issues").unwrap();
    let scope = sample_scope(InvocationId::new());
    let expected_url =
        "https://api.github.com/search/issues?q=repo%3Anearai%2Fironclaw%20is%3Aissue&per_page=1";
    let policy = github_policy();
    let network = RecordingNetworkHttpEgress::with_body(
        br#"{"total_count":0,"incomplete_results":false,"items":[]}"#.to_vec(),
    );
    let secret_store = Arc::new(InMemorySecretStore::new());
    let slot_handle = SecretHandle::new("github_runtime_token").unwrap();
    let account_access_secret = SecretHandle::new("github_manual_access").unwrap();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_github_package()),
        Arc::new(filesystem_with_github_package()),
        Arc::new(governor_with_default_limit(sample_account())),
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ApplyNetworkPolicy {
                policy: policy.clone(),
            },
            Obligation::InjectCredentialAccountOnce {
                handle: slot_handle,
                provider: RuntimeCredentialAccountProviderId::new("github").unwrap(),
                setup: ironclaw_host_api::RuntimeCredentialAccountSetup::ManualToken,
                provider_scopes: Vec::new(),
                requester_extension: ExtensionId::new("github").unwrap(),
            },
        ])),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_secret_store(Arc::clone(&secret_store))
    .with_runtime_credential_account_resolver(Arc::new(FixedRuntimeCredentialAccountResolver {
        result: Ok(account_access_secret.clone()),
    }))
    .with_trust_policy(Arc::new(github_first_party_trust_policy()))
    .try_with_host_http_egress(network.clone())
    .unwrap()
    .try_with_wasm_runtime(WitToolRuntimeConfig::default(), WitToolHost::deny_all())
    .unwrap();
    secret_store
        .put(
            scope.clone(),
            account_access_secret,
            SecretMaterial::from("ghp_fake_fixture_token"),
        )
        .await
        .unwrap();

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"repo": "nearai/ironclaw", "type": "issue", "limit": 1}),
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
async fn host_runtime_services_restages_github_product_auth_for_multi_request_wasm_capability() {
    let capability_id = CapabilityId::new("github.create_branch").unwrap();
    let scope = sample_scope(InvocationId::new());
    let source_sha = "abc123def4567890abc123def4567890abc123de";
    let policy = github_policy();
    let network = RecordingNetworkHttpEgress::with_body(
        format!(r#"{{"ref":"refs/heads/main","object":{{"sha":"{source_sha}"}}}}"#).into_bytes(),
    );
    let secret_store = Arc::new(InMemorySecretStore::new());
    let slot_handle = SecretHandle::new("github_runtime_token").unwrap();
    let account_access_secret = SecretHandle::new("github_manual_access").unwrap();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_github_package()),
        Arc::new(filesystem_with_github_package()),
        Arc::new(governor_with_default_limit(sample_account())),
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ApplyNetworkPolicy {
                policy: policy.clone(),
            },
            Obligation::InjectCredentialAccountOnce {
                handle: slot_handle,
                provider: RuntimeCredentialAccountProviderId::new("github").unwrap(),
                setup: ironclaw_host_api::RuntimeCredentialAccountSetup::ManualToken,
                provider_scopes: Vec::new(),
                requester_extension: ExtensionId::new("github").unwrap(),
            },
        ])),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_secret_store(Arc::clone(&secret_store))
    .with_runtime_credential_account_resolver(Arc::new(FixedRuntimeCredentialAccountResolver {
        result: Ok(account_access_secret.clone()),
    }))
    .with_trust_policy(Arc::new(github_first_party_trust_policy()))
    .try_with_host_http_egress(network.clone())
    .unwrap()
    .try_with_wasm_runtime(WitToolRuntimeConfig::default(), WitToolHost::deny_all())
    .unwrap();
    secret_store
        .put(
            scope.clone(),
            account_access_secret,
            SecretMaterial::from("ghp_fake_fixture_token"),
        )
        .await
        .unwrap();

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({
                "owner": "nearai",
                "repo": "ironclaw",
                "branch": "feature/matrix",
                "from_ref": "main"
            }),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id);
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
    let requests = network.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, NetworkMethod::Get);
    assert_eq!(
        requests[0].url,
        "https://api.github.com/repos/nearai/ironclaw/git/ref/heads/main"
    );
    assert_eq!(requests[1].method, NetworkMethod::Post);
    assert_eq!(
        requests[1].url,
        "https://api.github.com/repos/nearai/ironclaw/git/refs"
    );
    for request in &requests {
        assert_eq!(request.policy, policy);
        assert_google_bearer_header(request, "ghp_fake_fixture_token");
    }
    let create_body: serde_json::Value =
        serde_json::from_slice(&requests[1].body).expect("create branch JSON body");
    assert_eq!(
        create_body,
        json!({"ref": "refs/heads/feature/matrix", "sha": source_sha})
    );
}

#[tokio::test]
async fn host_runtime_services_routes_google_drive_wasm_list_files_with_scoped_google_credential() {
    let capability_id = CapabilityId::new("google-drive.list_files").unwrap();
    let scope = sample_scope(InvocationId::new());
    let policy = google_drive_policy();
    let network = RecordingNetworkHttpEgress::with_body(br#"{"files":[]}"#.to_vec());
    let secret_store = Arc::new(InMemorySecretStore::new());
    let slot_handle = SecretHandle::new("google_runtime_token").unwrap();
    let account_access_secret = SecretHandle::new("google_manual_access").unwrap();
    let required_scopes = vec!["https://www.googleapis.com/auth/drive.readonly".to_string()];
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_google_drive_package()),
        Arc::new(filesystem_with_google_drive_package()),
        Arc::new(governor_with_default_limit(sample_account())),
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ApplyNetworkPolicy {
                policy: policy.clone(),
            },
            Obligation::InjectCredentialAccountOnce {
                handle: slot_handle,
                provider: RuntimeCredentialAccountProviderId::new("google").unwrap(),
                setup: ironclaw_host_api::RuntimeCredentialAccountSetup::OAuth {
                    scopes: required_scopes.clone(),
                },
                provider_scopes: required_scopes.clone(),
                requester_extension: ExtensionId::new("google-drive").unwrap(),
            },
        ])),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_secret_store(Arc::clone(&secret_store))
    .with_runtime_credential_account_resolver(Arc::new(
        FixedGoogleRuntimeCredentialAccountResolver {
            expected_requester_extension: ExtensionId::new("google-drive").unwrap(),
            expected_scopes: required_scopes,
            result: Ok(account_access_secret.clone()),
        },
    ))
    .with_trust_policy(Arc::new(google_drive_first_party_trust_policy()))
    .try_with_host_http_egress(network.clone())
    .unwrap()
    .try_with_wasm_runtime(WitToolRuntimeConfig::default(), WitToolHost::deny_all())
    .unwrap();
    secret_store
        .put(
            scope.clone(),
            account_access_secret,
            SecretMaterial::from("ya29.fake_fixture_token"),
        )
        .await
        .unwrap();

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"query": "name contains 'report'"}),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id);
            assert_eq!(completed.output, json!({"files":[]}));
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
    let requests = network.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, NetworkMethod::Get);
    assert!(
        requests[0]
            .url
            .starts_with("https://www.googleapis.com/drive/v3/files?")
    );
    assert!(requests[0].url.contains("pageSize=25"));
    assert!(requests[0].url.contains("q=name%20contains%20%27report%27"));
    assert_eq!(requests[0].body, Vec::<u8>::new());
    assert_eq!(requests[0].policy, policy);
    assert_eq!(
        requests[0]
            .headers
            .iter()
            .find(|(name, _)| name == "authorization"),
        Some(&(
            "authorization".to_string(),
            "Bearer ya29.fake_fixture_token".to_string(),
        ))
    );
}

#[tokio::test]
async fn host_runtime_services_routes_google_docs_wasm_get_document_with_scoped_google_credential()
{
    let capability_id = CapabilityId::new("google-docs.get_document").unwrap();
    let scope = sample_scope(InvocationId::new());
    let policy = google_policy("docs.googleapis.com");
    let network = RecordingNetworkHttpEgress::with_body(
        br#"{"documentId":"doc-1","title":"Doc","revisionId":"r1","body":{"content":[{"endIndex":5}]}}"#.to_vec(),
    );
    let secret_store = Arc::new(InMemorySecretStore::new());
    let account_access_secret = SecretHandle::new("google_docs_access").unwrap();
    let required_scopes = vec!["https://www.googleapis.com/auth/documents.readonly".to_string()];
    let services = google_wasm_services_for_test!(
        "google-docs",
        policy.clone(),
        network.clone(),
        Arc::clone(&secret_store),
        account_access_secret.clone(),
        required_scopes,
    );
    secret_store
        .put(
            scope.clone(),
            account_access_secret,
            SecretMaterial::from("ya29.fake_fixture_token"),
        )
        .await
        .unwrap();

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"document_id": "doc-1"}),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id);
            assert_eq!(completed.output["document_id"], json!("doc-1"));
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
    let requests = network.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, NetworkMethod::Get);
    assert_eq!(
        requests[0].url,
        "https://docs.googleapis.com/v1/documents/doc-1"
    );
    assert_eq!(requests[0].body, Vec::<u8>::new());
    assert_eq!(requests[0].policy, policy);
    assert_google_bearer_header(&requests[0], "ya29.fake_fixture_token");
}

#[tokio::test]
async fn host_runtime_services_routes_google_sheets_wasm_get_spreadsheet_with_scoped_google_credential()
 {
    let capability_id = CapabilityId::new("google-sheets.get_spreadsheet").unwrap();
    let scope = sample_scope(InvocationId::new());
    let policy = google_policy("sheets.googleapis.com");
    let network = RecordingNetworkHttpEgress::with_body(
        br#"{"spreadsheetId":"sheet-1","properties":{"title":"Sheet"},"spreadsheetUrl":"https://docs.google.com/spreadsheets/d/sheet-1","sheets":[],"namedRanges":[]}"#.to_vec(),
    );
    let secret_store = Arc::new(InMemorySecretStore::new());
    let account_access_secret = SecretHandle::new("google_sheets_access").unwrap();
    let required_scopes = vec!["https://www.googleapis.com/auth/spreadsheets.readonly".to_string()];
    let services = google_wasm_services_for_test!(
        "google-sheets",
        policy.clone(),
        network.clone(),
        Arc::clone(&secret_store),
        account_access_secret.clone(),
        required_scopes,
    );
    secret_store
        .put(
            scope.clone(),
            account_access_secret,
            SecretMaterial::from("ya29.fake_fixture_token"),
        )
        .await
        .unwrap();

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"spreadsheet_id": "sheet-1"}),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id);
            assert_eq!(completed.output["spreadsheet_id"], json!("sheet-1"));
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
    let requests = network.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, NetworkMethod::Get);
    assert_eq!(
        requests[0].url,
        "https://sheets.googleapis.com/v4/spreadsheets/sheet-1?fields=spreadsheetId,properties.title,spreadsheetUrl,sheets.properties,namedRanges"
    );
    assert_eq!(requests[0].body, Vec::<u8>::new());
    assert_eq!(requests[0].policy, policy);
    assert_google_bearer_header(&requests[0], "ya29.fake_fixture_token");
}

#[tokio::test]
async fn host_runtime_services_routes_google_slides_wasm_get_presentation_with_scoped_google_credential()
 {
    let capability_id = CapabilityId::new("google-slides.get_presentation").unwrap();
    let scope = sample_scope(InvocationId::new());
    let policy = google_policy("slides.googleapis.com");
    let network = RecordingNetworkHttpEgress::with_body(
        br#"{"presentationId":"slides-1","title":"Slides","revisionId":"r1","slides":[]}"#.to_vec(),
    );
    let secret_store = Arc::new(InMemorySecretStore::new());
    let account_access_secret = SecretHandle::new("google_slides_access").unwrap();
    let required_scopes =
        vec!["https://www.googleapis.com/auth/presentations.readonly".to_string()];
    let services = google_wasm_services_for_test!(
        "google-slides",
        policy.clone(),
        network.clone(),
        Arc::clone(&secret_store),
        account_access_secret.clone(),
        required_scopes,
    );
    secret_store
        .put(
            scope.clone(),
            account_access_secret,
            SecretMaterial::from("ya29.fake_fixture_token"),
        )
        .await
        .unwrap();

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"presentation_id": "slides-1"}),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id);
            assert_eq!(completed.output["presentation_id"], json!("slides-1"));
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
    let requests = network.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, NetworkMethod::Get);
    assert_eq!(
        requests[0].url,
        "https://slides.googleapis.com/v1/presentations/slides-1"
    );
    assert_eq!(requests[0].body, Vec::<u8>::new());
    assert_eq!(requests[0].policy, policy);
    assert_google_bearer_header(&requests[0], "ya29.fake_fixture_token");
}

#[tokio::test]
async fn host_runtime_services_maps_github_wasm_input_errors_to_invalid_input() {
    let capability_id = CapabilityId::new("github.search_issues").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = RecordingNetworkHttpEgress::with_body(
        br#"{"total_count":0,"incomplete_results":false,"items":[]}"#.to_vec(),
    );
    let secret_store = Arc::new(InMemorySecretStore::new());
    let slot_handle = SecretHandle::new("github_runtime_token").unwrap();
    let account_access_secret = SecretHandle::new("github_manual_access").unwrap();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_github_package()),
        Arc::new(filesystem_with_github_package()),
        Arc::new(governor_with_default_limit(sample_account())),
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ApplyNetworkPolicy {
                policy: github_policy(),
            },
            Obligation::InjectCredentialAccountOnce {
                handle: slot_handle,
                provider: RuntimeCredentialAccountProviderId::new("github").unwrap(),
                setup: ironclaw_host_api::RuntimeCredentialAccountSetup::ManualToken,
                provider_scopes: Vec::new(),
                requester_extension: ExtensionId::new("github").unwrap(),
            },
        ])),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_secret_store(Arc::clone(&secret_store))
    .with_runtime_credential_account_resolver(Arc::new(FixedRuntimeCredentialAccountResolver {
        result: Ok(account_access_secret.clone()),
    }))
    .with_trust_policy(Arc::new(github_first_party_trust_policy()))
    .try_with_host_http_egress(network.clone())
    .unwrap()
    .try_with_wasm_runtime(WitToolRuntimeConfig::default(), WitToolHost::deny_all())
    .unwrap();
    secret_store
        .put(
            scope.clone(),
            account_access_secret,
            SecretMaterial::from("ghp_fake_fixture_token"),
        )
        .await
        .unwrap();

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id,
            scope,
            json!({}),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, RuntimeFailureKind::InvalidInput);
    assert!(
        network.requests().is_empty(),
        "guest validation failures must block before HTTP egress"
    );
}

#[tokio::test]
async fn host_runtime_services_missing_github_runtime_secret_blocks_on_auth() {
    let capability_id = CapabilityId::new("github.search_issues").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = RecordingNetworkHttpEgress::with_body(
        br#"{"total_count":0,"incomplete_results":false,"items":[]}"#.to_vec(),
    );
    let secret_store = Arc::new(InMemorySecretStore::new());
    let slot_handle = SecretHandle::new("github_runtime_token").unwrap();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_github_package()),
        Arc::new(filesystem_with_github_package()),
        Arc::new(governor_with_default_limit(sample_account())),
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ApplyNetworkPolicy {
                policy: github_policy(),
            },
            Obligation::InjectCredentialAccountOnce {
                handle: slot_handle,
                provider: RuntimeCredentialAccountProviderId::new("github").unwrap(),
                setup: ironclaw_host_api::RuntimeCredentialAccountSetup::ManualToken,
                provider_scopes: Vec::new(),
                requester_extension: ExtensionId::new("github").unwrap(),
            },
        ])),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_secret_store(Arc::clone(&secret_store))
    .with_runtime_credential_account_resolver(Arc::new(FixedRuntimeCredentialAccountResolver {
        result: Err(CredentialStageError::AuthRequired),
    }))
    .with_trust_policy(Arc::new(github_first_party_trust_policy()))
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
async fn bundled_github_wasm_builds_query_from_structured_search_fields() {
    let http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 200,
        headers_json: "{}".to_string(),
        body: br#"{"total_count":0,"incomplete_results":false,"items":[]}"#.to_vec(),
    }));
    let execution = execute_bundled_github_wasm(
        "github.search_issues",
        json!({
            "repo": "nearai/ironclaw",
            "author": "serrrfirat",
            "type": "issue",
            "state": "open",
            "limit": 1
        }),
        Arc::clone(&http),
    );

    assert_eq!(execution.error, None);
    assert_single_wasm_request(
        &http,
        "GET",
        "https://api.github.com/search/issues?q=repo%3Anearai%2Fironclaw%20author%3Aserrrfirat%20state%3Aopen%20is%3Aissue&per_page=1",
        None,
    );
}

#[tokio::test]
async fn bundled_github_wasm_replies_to_pull_request_comment_under_pr_path() {
    let http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 201,
        headers_json: "{}".to_string(),
        body: br##"{"id":45,"body":"Reply from Reborn"}"##.to_vec(),
    }));

    let reply = execute_bundled_github_wasm(
        "github.reply_pull_request_comment",
        json!({
            "owner": "nearai",
            "repo": "ironclaw",
            "pr_number": 4280,
            "comment_id": 123456789_u64,
            "body": "Reply from Reborn",
        }),
        Arc::clone(&http),
    );

    assert_eq!(reply.error, None);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(reply.output_json.as_deref().unwrap()).unwrap()["body"],
        json!("Reply from Reborn")
    );
    assert_single_wasm_request(
        &http,
        "POST",
        "https://api.github.com/repos/nearai/ironclaw/pulls/4280/comments/123456789/replies",
        Some(br#"{"body":"Reply from Reborn"}"#),
    );
}

#[tokio::test]
async fn bundled_github_wasm_returns_json_for_empty_success_responses() {
    let http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 204,
        headers_json: "{}".to_string(),
        body: Vec::new(),
    }));

    let dispatch = execute_bundled_github_wasm(
        "github.trigger_workflow",
        json!({
            "owner": "nearai",
            "repo": "ironclaw",
            "workflow_id": "ci.yml",
            "ref": "main",
            "inputs": {"suite": "smoke"}
        }),
        Arc::clone(&http),
    );

    assert_eq!(dispatch.error, None);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(dispatch.output_json.as_deref().unwrap())
            .unwrap(),
        json!({"status": 204})
    );
    assert_single_wasm_request(
        &http,
        "POST",
        "https://api.github.com/repos/nearai/ironclaw/actions/workflows/ci.yml/dispatches",
        Some(br#"{"inputs":{"suite":"smoke"},"ref":"main"}"#),
    );
}

#[tokio::test]
async fn bundled_github_wasm_create_branch_rejects_source_ref_without_sha() {
    let http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 200,
        headers_json: "{}".to_string(),
        body: br#"{"object":{"type":"commit"}}"#.to_vec(),
    }));

    let create = execute_bundled_github_wasm(
        "github.create_branch",
        json!({
            "owner": "nearai",
            "repo": "ironclaw",
            "branch": "feature/reborn-github",
            "from_ref": "main"
        }),
        Arc::clone(&http),
    );

    assert_eq!(
        structured_wasm_error_code(&create).as_deref(),
        Some("Source ref response missing object.sha")
    );
    let requests = http.requests().unwrap();
    assert_eq!(
        requests.len(),
        1,
        "malformed source ref response must not create the branch ref"
    );
    assert_eq!(
        requests[0].url,
        "https://api.github.com/repos/nearai/ironclaw/git/ref/heads/main"
    );
}

#[tokio::test]
async fn bundled_github_wasm_create_branch_propagates_missing_source_ref() {
    let http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 404,
        headers_json: "{}".to_string(),
        body: br#"{"message":"Not Found"}"#.to_vec(),
    }));

    let create = execute_bundled_github_wasm(
        "github.create_branch",
        json!({
            "owner": "nearai",
            "repo": "ironclaw",
            "branch": "feature/reborn-github",
            "from_ref": "missing-branch"
        }),
        Arc::clone(&http),
    );

    assert_eq!(
        structured_wasm_error_code(&create).as_deref(),
        Some("github_api_error_status_404")
    );
    let requests = http.requests().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url,
        "https://api.github.com/repos/nearai/ironclaw/git/ref/heads/missing-branch"
    );
}

#[tokio::test]
async fn bundled_github_wasm_rejects_raw_sha_as_create_branch_source_ref() {
    let http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 200,
        headers_json: "{}".to_string(),
        body: br#"{"object":{"sha":"abc"}}"#.to_vec(),
    }));

    let create = execute_bundled_github_wasm(
        "github.create_branch",
        json!({
            "owner": "nearai",
            "repo": "ironclaw",
            "branch": "feature/reborn-github",
            "from_ref": "0123456789abcdef0123456789abcdef01234567"
        }),
        Arc::clone(&http),
    );

    assert_eq!(
        structured_wasm_error_code(&create).as_deref(),
        Some("Unsupported from_ref: use a branch or tag ref, not a raw commit SHA")
    );
    assert!(
        http.requests().unwrap().is_empty(),
        "raw SHA validation should fail before GitHub egress"
    );
}

#[tokio::test]
async fn bundled_github_wasm_builds_create_repo_fork_and_release_requests() {
    let create_repo_http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 201,
        headers_json: "{}".to_string(),
        body: br#"{"name":"reborn-fixture"}"#.to_vec(),
    }));
    let create_repo = execute_bundled_github_wasm(
        "github.create_repo",
        json!({
            "name": "reborn-fixture",
            "description": "fixture repo",
            "private": true,
            "auto_init": true
        }),
        Arc::clone(&create_repo_http),
    );
    assert_eq!(create_repo.error, None);
    assert_single_wasm_request_json_body(
        &create_repo_http,
        "POST",
        "https://api.github.com/user/repos",
        json!({
            "name": "reborn-fixture",
            "description": "fixture repo",
            "private": true,
            "auto_init": true
        }),
    );

    let fork_http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 202,
        headers_json: "{}".to_string(),
        body: br#"{"name":"ironclaw-fork"}"#.to_vec(),
    }));
    let fork = execute_bundled_github_wasm(
        "github.fork_repo",
        json!({
            "owner": "nearai",
            "repo": "ironclaw",
            "organization": "nearai-labs",
            "name": "ironclaw-fork",
            "default_branch_only": true
        }),
        Arc::clone(&fork_http),
    );
    assert_eq!(fork.error, None);
    assert_single_wasm_request_json_body(
        &fork_http,
        "POST",
        "https://api.github.com/repos/nearai/ironclaw/forks",
        json!({
            "organization": "nearai-labs",
            "name": "ironclaw-fork",
            "default_branch_only": true
        }),
    );

    let release_http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 201,
        headers_json: "{}".to_string(),
        body: br#"{"tag_name":"v1.2.3"}"#.to_vec(),
    }));
    let release = execute_bundled_github_wasm(
        "github.create_release",
        json!({
            "owner": "nearai",
            "repo": "ironclaw",
            "tag_name": "v1.2.3",
            "target_commitish": "main",
            "name": "v1.2.3",
            "body": "release notes",
            "draft": true,
            "prerelease": false,
            "generate_release_notes": true
        }),
        Arc::clone(&release_http),
    );
    assert_eq!(release.error, None);
    assert_single_wasm_request_json_body(
        &release_http,
        "POST",
        "https://api.github.com/repos/nearai/ironclaw/releases",
        json!({
            "tag_name": "v1.2.3",
            "target_commitish": "main",
            "name": "v1.2.3",
            "body": "release notes",
            "draft": true,
            "prerelease": false,
            "generate_release_notes": true
        }),
    );
}

#[tokio::test]
async fn bundled_github_wasm_rejects_relative_file_path_segments_before_egress() {
    let http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 200,
        headers_json: "{}".to_string(),
        body: br#"{"content":"Zm9v"}"#.to_vec(),
    }));
    let file = execute_bundled_github_wasm(
        "github.get_file_content",
        json!({
            "owner": "nearai",
            "repo": "ironclaw",
            "path": "src/./main.rs"
        }),
        Arc::clone(&http),
    );

    assert_eq!(
        structured_wasm_error_code(&file).as_deref(),
        Some("Invalid path: relative path segments not allowed")
    );
    assert!(
        http.requests().unwrap().is_empty(),
        "relative path segment validation should fail before GitHub egress"
    );
}

#[tokio::test]
async fn bundled_github_wasm_rejects_invalid_review_event_and_merge_method() {
    let review_http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 200,
        headers_json: "{}".to_string(),
        body: br#"{"id":1}"#.to_vec(),
    }));
    let review = execute_bundled_github_wasm(
        "github.create_pr_review",
        json!({
            "owner": "nearai",
            "repo": "ironclaw",
            "pr_number": 4280,
            "body": "review body",
            "event": "approve"
        }),
        Arc::clone(&review_http),
    );
    assert_eq!(
        structured_wasm_error_code(&review).as_deref(),
        Some("invalid_parameters")
    );
    assert!(
        review_http.requests().unwrap().is_empty(),
        "invalid review event should fail before GitHub egress"
    );

    let merge_http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 200,
        headers_json: "{}".to_string(),
        body: br#"{"merged":true}"#.to_vec(),
    }));
    let merge = execute_bundled_github_wasm(
        "github.merge_pull_request",
        json!({
            "owner": "nearai",
            "repo": "ironclaw",
            "pr_number": 4280,
            "merge_method": "fast-forward"
        }),
        Arc::clone(&merge_http),
    );
    assert_eq!(
        structured_wasm_error_code(&merge).as_deref(),
        Some("invalid_parameters")
    );
    assert!(
        merge_http.requests().unwrap().is_empty(),
        "invalid merge method should fail before GitHub egress"
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
        assert_eq!(
            structured_wasm_error_code(&execution).as_deref(),
            Some(expected_error)
        );
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

#[test]
fn bundled_google_drive_wasm_rejects_invalid_context_derived_dispatch_inputs() {
    let http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 200,
        headers_json: "{}".to_string(),
        body: br#"{"files":[]}"#.to_vec(),
    }));

    let missing_context = execute_bundled_google_drive_wasm(json!({}), None, Arc::clone(&http));
    assert_eq!(
        wasm_error_code_or_text(&missing_context).as_deref(),
        Some("missing_invocation_context")
    );

    let malformed_context =
        execute_bundled_google_drive_wasm(json!({}), Some("not-json"), Arc::clone(&http));
    assert_eq!(
        wasm_error_code_or_text(&malformed_context).as_deref(),
        Some("invalid_invocation_context")
    );

    let unsupported_capability = execute_bundled_google_drive_wasm(
        json!({}),
        Some(r#"{"capability_id":"google-drive.nope"}"#),
        Arc::clone(&http),
    );
    assert_eq!(
        wasm_error_code_or_text(&unsupported_capability).as_deref(),
        Some("unsupported_google_drive_capability")
    );

    let action_collision = execute_bundled_google_drive_wasm(
        json!({"action": "list_files"}),
        Some(r#"{"capability_id":"google-drive.list_files"}"#),
        Arc::clone(&http),
    );
    assert_eq!(
        wasm_error_code_or_text(&action_collision).as_deref(),
        Some("invalid_parameters")
    );

    assert!(
        http.requests().unwrap().is_empty(),
        "dispatch-wrapper validation failures must block before HTTP egress"
    );
}

fn assert_failed_outcome(outcome: RuntimeCapabilityOutcome, expected_kind: RuntimeFailureKind) {
    match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => assert_eq!(failure.kind, expected_kind),
        other => panic!("expected failed outcome {expected_kind:?}, got {other:?}"),
    }
}

fn structured_wasm_error_code(execution: &WitToolExecution) -> Option<String> {
    let error = execution.error.as_deref()?;
    let parsed: serde_json::Value =
        serde_json::from_str(error).expect("WASM guest errors are structured JSON");
    assert!(
        parsed["kind"].as_str().is_some_and(|kind| !kind.is_empty()),
        "structured WASM guest error must include a non-empty kind"
    );
    parsed["code"].as_str().map(str::to_string)
}

fn wasm_error_code_or_text(execution: &WitToolExecution) -> Option<String> {
    let error = execution.error.as_deref()?;
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(error)
        && let Some(code) = parsed["code"].as_str()
    {
        return Some(code.to_string());
    }
    Some(error.to_string())
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

#[derive(Debug)]
struct FixedRuntimeCredentialAccountResolver {
    result: Result<SecretHandle, CredentialStageError>,
}

#[async_trait]
impl RuntimeCredentialAccountResolver for FixedRuntimeCredentialAccountResolver {
    async fn resolve_access_secret(
        &self,
        request: RuntimeCredentialAccountRequest<'_>,
    ) -> Result<RuntimeCredentialAccessSecret, CredentialStageError> {
        assert_eq!(request.provider.as_str(), "github");
        assert_eq!(request.requester_extension.as_str(), "github");
        self.result
            .clone()
            .map(|handle| RuntimeCredentialAccessSecret {
                scope: request.scope.clone(),
                handle,
            })
    }
}

#[derive(Debug)]
struct FixedGoogleRuntimeCredentialAccountResolver {
    expected_requester_extension: ExtensionId,
    expected_scopes: Vec<String>,
    result: Result<SecretHandle, CredentialStageError>,
}

#[async_trait]
impl RuntimeCredentialAccountResolver for FixedGoogleRuntimeCredentialAccountResolver {
    async fn resolve_access_secret(
        &self,
        request: RuntimeCredentialAccountRequest<'_>,
    ) -> Result<RuntimeCredentialAccessSecret, CredentialStageError> {
        assert_eq!(request.provider.as_str(), "google");
        assert_eq!(
            request.requester_extension,
            &self.expected_requester_extension
        );
        assert_eq!(request.provider_scopes, self.expected_scopes.as_slice());
        self.result
            .clone()
            .map(|handle| RuntimeCredentialAccessSecret {
                scope: request.scope.clone(),
                handle,
            })
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

fn registry_with_google_drive_package() -> ExtensionRegistry {
    registry_with_google_package("google-drive")
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

fn filesystem_with_google_drive_package() -> LocalFilesystem {
    filesystem_with_google_package("google-drive")
}

fn registry_with_google_package(package_id: &str) -> ExtensionRegistry {
    let manifest = ExtensionManifest::parse_with_host_api_contracts(
        &std::fs::read_to_string(google_asset_root(package_id).join("manifest.toml")).unwrap(),
        ManifestSource::HostBundled,
        &default_host_port_catalog().unwrap(),
        &default_host_api_contract_registry().unwrap(),
    )
    .unwrap();
    let package = ExtensionPackage::from_manifest(
        manifest,
        VirtualPath::new(format!("/system/extensions/{package_id}")).unwrap(),
    )
    .unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
}

fn filesystem_with_google_package(package_id: &str) -> LocalFilesystem {
    let mut filesystem = LocalFilesystem::new();
    filesystem
        .mount_local(
            VirtualPath::new("/system/extensions").unwrap(),
            HostPath::from_path_buf(
                google_asset_root(package_id)
                    .parent()
                    .unwrap()
                    .to_path_buf(),
            ),
        )
        .unwrap();
    filesystem
}

fn github_asset_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("crates/ironclaw_first_party_extensions/assets/github")
}

fn google_drive_asset_root() -> std::path::PathBuf {
    google_asset_root("google-drive")
}

fn google_asset_root(package_id: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("crates/ironclaw_first_party_extensions/assets")
        .join(package_id)
}

fn github_wasm_path() -> std::path::PathBuf {
    github_asset_root().join("wasm/github_tool.wasm")
}

fn google_drive_wasm_path() -> std::path::PathBuf {
    google_drive_asset_root().join("wasm/google_drive_tool.wasm")
}

fn google_drive_policy() -> NetworkPolicy {
    google_policy("www.googleapis.com")
}

fn google_policy(host_pattern: &str) -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: host_pattern.to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(10_000),
    }
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

fn google_drive_first_party_trust_policy() -> HostTrustPolicy {
    google_first_party_trust_policy("google-drive")
}

fn google_first_party_trust_policy(package_id: &str) -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new(package_id).unwrap(),
            format!("/system/extensions/{package_id}/manifest.toml"),
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

fn assert_google_bearer_header(request: &NetworkHttpRequest, expected_token: &str) {
    assert_eq!(
        request
            .headers
            .iter()
            .find(|(name, _)| name == "authorization"),
        Some(&(
            "authorization".to_string(),
            format!("Bearer {expected_token}"),
        ))
    );
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
            secrets: Vec::new(),
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

fn execute_bundled_google_drive_wasm(
    input: serde_json::Value,
    context: Option<&str>,
    http: Arc<RecordingWasmHostHttp>,
) -> WitToolExecution {
    let runtime = WitToolRuntime::new(WitToolRuntimeConfig::default()).unwrap();
    let wasm_bytes = std::fs::read(google_drive_wasm_path())
        .expect("first-party Google Drive WASM must be built");
    let prepared = runtime.prepare("google-drive", &wasm_bytes).unwrap();
    let request = match context {
        Some(context) => WitToolRequest::new(input.to_string()).with_context(context.to_string()),
        None => WitToolRequest::new(input.to_string()),
    };
    runtime
        .execute(&prepared, WitToolHost::deny_all().with_http(http), request)
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

fn assert_single_wasm_request_json_body(
    http: &RecordingWasmHostHttp,
    expected_method: &str,
    expected_url: &str,
    expected_body: serde_json::Value,
) {
    let requests = http.requests().unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, expected_method);
    assert_eq!(request.url, expected_url);
    assert_eq!(request.timeout_ms, Some(10_000));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(request.body.as_deref().unwrap()).unwrap(),
        expected_body
    );

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
