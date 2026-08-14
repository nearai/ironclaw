// arch-exempt: large_file, mechanical DiskFilesystem->DiskFilesystem Bucket-2 rename (arch-simplification §4.4), no logic change, plan #6168
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer;
use ironclaw_extension_registry::{
    ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource,
    default_host_api_contract_registry,
};
use ironclaw_filesystem::DiskFilesystem;
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_host_api::result_meta::FailureKind;
use ironclaw_host_api::{
    action::{NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern},
    capability::{
        CapabilityDescriptor, CapabilityGrant, CapabilitySet, EffectKind, GrantConstraints,
    },
    decision::{Decision, Obligation, Obligations},
    dispatch::CredentialStageError,
    host_port::default_host_port_catalog,
    ids::{
        AgentId, CapabilityGrantId, CapabilityId, CorrelationId, ExtensionId, InvocationId,
        MissionId, PackageId, ProjectId, RunId, SecretHandle, TenantId, UserId, VendorId,
    },
    mount::MountView,
    path::{HostPath, VirtualPath},
    resource::{ResourceEstimate, ResourceScope},
    runtime::{RuntimeKind, TrustClass},
    scope::{ExecutionContext, Principal},
};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, HostRuntime, HostRuntimeServices, RuntimeCapabilityFailure,
    RuntimeCapabilityOutcome, RuntimeCredentialAccessSecret, RuntimeCredentialAccountRequest,
    RuntimeCredentialAccountResolver, RuntimeInvocation,
};
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
};
use ironclaw_processes::ProcessServices;
use ironclaw_resources::{
    InMemoryResourceGovernor, ResourceAccount, ResourceGovernor, ResourceLimits,
};
use ironclaw_secrets::{SecretMaterial, SecretStore, SecretStorePort};
use ironclaw_trust::{
    AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy, TrustDecision,
};
use ironclaw_wasm::{
    PreparedWitTool, RecordingWasmHostHttp, WasmHostError, WasmHttpResponse, WitToolExecution,
    WitToolHost, WitToolRequest, WitToolRuntime, WitToolRuntimeConfig,
};
use serde_json::json;

macro_rules! github_wasm_services_for_test {
    (
        $network:expr,
        $secret_store:expr,
        $account_access_secret:expr $(,)?
    ) => {{
        HostRuntimeServices::new(
            Arc::new(registry_with_github_package()),
            Arc::new(filesystem_with_github_package()),
            Arc::new(governor_with_default_limit(sample_account())),
            Arc::new(ObligatingAuthorizer::new(vec![
                Obligation::ApplyNetworkPolicy {
                    policy: github_policy(),
                },
                Obligation::InjectCredentialAccountOnce {
                    handle: SecretHandle::new("github_runtime_token").unwrap(),
                    provider: VendorId::new("github").unwrap(),
                    setup:
                        ironclaw_host_api::capability::RuntimeCredentialAccountSetup::ManualToken,
                    provider_scopes: Vec::new(),
                    requester_extension: ExtensionId::new("github").unwrap(),
                },
            ])),
            ProcessServices::in_memory(),
            CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        )
        .with_secret_store($secret_store)
        .with_runtime_credential_account_resolver(Arc::new(FixedRuntimeCredentialAccountResolver {
            result: Ok($account_access_secret),
        }))
        .with_trust_policy(Arc::new(github_first_party_trust_policy()))
        .try_with_host_http_egress($network)
        .unwrap()
        .try_with_wasm_runtime(WitToolRuntimeConfig::default(), WitToolHost::deny_all())
        .unwrap()
    }};
}

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
                    provider: VendorId::new("google").unwrap(),
                    setup: ironclaw_host_api::capability::RuntimeCredentialAccountSetup::OAuth {
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
async fn host_runtime_services_compact_github_search_preserves_pagination_and_egress() {
    let capability_id = CapabilityId::new("github.search_issues_pull_requests").unwrap();
    let scope = sample_scope(InvocationId::new());
    let expected_url = "https://api.github.com/search/issues?q=repo%3Anearai%2Fironclaw%20is%3Apr&per_page=30&page=2";
    let policy = github_policy();
    let large_body = "x".repeat(24 * 1024);
    let items = (0..30)
        .map(|index| {
            json!({
                "number": 7_000 + index,
                "title": format!("Prioritize search result {index}"),
                "body": large_body,
                "state": "open",
                "draft": false,
                "html_url": format!("https://github.com/nearai/ironclaw/pull/{}", 7_000 + index),
                "repository_url": "https://api.github.com/repos/nearai/ironclaw",
                "user": {"login": format!("author-{index}"), "avatar_url": "https://example.test/avatar"},
                "labels": [{"name": "priority/high", "description": large_body}],
                "assignees": [{"login": "review-owner", "avatar_url": "https://example.test/avatar"}],
                "comments": 23,
                "created_at": "2026-07-01T00:00:00Z",
                "updated_at": "2026-07-31T00:00:00Z",
                "pull_request": {
                    "url": format!("https://api.github.com/repos/nearai/ironclaw/pulls/{}", 7_000 + index),
                    "html_url": format!("https://github.com/nearai/ironclaw/pull/{}", 7_000 + index),
                    "diff_url": "https://example.test/large.diff"
                },
                "reactions": {"url": "https://api.github.com/large"},
                "score": 1.0
            })
        })
        .collect::<Vec<_>>();
    let provider_body = json!({
        "total_count": 5_000,
        "incomplete_results": false,
        "items": items
    })
    .to_string();
    assert!(provider_body.len() > 1024 * 1024);
    let network = RecordingNetworkHttpEgress::with_body(provider_body.into_bytes());
    let secret_store = Arc::new(SecretStore::ephemeral());
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
                provider: VendorId::new("github").unwrap(),
                setup: ironclaw_host_api::capability::RuntimeCredentialAccountSetup::ManualToken,
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
            None,
        )
        .await
        .unwrap();

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"repo": "nearai/ironclaw", "type": "pr", "limit": 30, "page": 2}),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id);
            assert_eq!(completed.output["total_count"], 5_000);
            assert_eq!(completed.output["items"].as_array().map(Vec::len), Some(30));
            assert_eq!(
                completed.output["items"][0]["user"],
                json!({"login": "author-0"})
            );
            assert!(completed.output["items"][0].get("body").is_none());
            assert!(completed.output["items"][0].get("reactions").is_none());
            assert!(
                completed.output.to_string().len() < 30 * 1024,
                "host-visible search output should remain compact"
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
async fn host_runtime_services_compact_github_pull_list_preserves_page_shape() {
    let capability_id = CapabilityId::new("github.list_pull_requests").unwrap();
    let scope = sample_scope(InvocationId::new());
    let large_body = "x".repeat(16 * 1024);
    let provider_items = (0..30)
        .map(|index| {
            json!({
                "number": 8_000 + index,
                "title": format!("Prioritize pull request {index}"),
                "body": large_body,
                "state": "open",
                "draft": index % 2 == 0,
                "html_url": format!("https://github.com/nearai/ironclaw/pull/{}", 8_000 + index),
                "user": {"login": format!("author-{index}"), "avatar_url": "https://example.test/avatar"},
                "labels": [{"name": "priority/high", "description": large_body}],
                "assignees": [{"login": "review-owner", "avatar_url": "https://example.test/avatar"}],
                "requested_reviewers": [{"login": "maintainer", "avatar_url": "https://example.test/avatar"}],
                "requested_teams": [{"slug": "core", "description": large_body}],
                "created_at": "2026-07-01T00:00:00Z",
                "updated_at": "2026-07-31T00:00:00Z",
                "head": {
                    "ref": format!("feature/{index}"),
                    "sha": format!("{index:040x}"),
                    "repo": {"full_name": "nearai/ironclaw", "description": large_body}
                },
                "base": {
                    "ref": "main",
                    "sha": "1111111111111111111111111111111111111111",
                    "repo": {"full_name": "nearai/ironclaw", "description": large_body}
                },
                "_links": {"self": {"href": "https://api.github.com/large"}}
            })
        })
        .collect::<Vec<_>>();
    let provider_body = serde_json::to_string(&provider_items).expect("provider fixture JSON");
    assert!(provider_body.len() > 2 * 1024 * 1024);
    let network = RecordingNetworkHttpEgress::with_body(provider_body.into_bytes());
    let secret_store = Arc::new(SecretStore::ephemeral());
    let account_access_secret = SecretHandle::new("github_manual_access").unwrap();
    let services = github_wasm_services_for_test!(
        network.clone(),
        Arc::clone(&secret_store),
        account_access_secret.clone(),
    );
    secret_store
        .put(
            scope.clone(),
            account_access_secret,
            SecretMaterial::from("ghp_fake_fixture_token"),
            None,
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
                "page": 3,
                "limit": 30
            }),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id);
            let items = completed.output.as_array().expect("list remains an array");
            assert_eq!(items.len(), 30);
            assert_eq!(items[0]["number"], 8_000);
            assert_eq!(items[0]["labels"], json!([{"name": "priority/high"}]));
            assert!(items[0].get("body").is_none());
            assert!(items[0]["head"].get("repo").is_none());
            assert!(
                completed.output.to_string().len() < 30 * 1024,
                "host-visible pull list output should remain compact"
            );
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
    let requests = network.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url,
        "https://api.github.com/repos/nearai/ironclaw/pulls?state=open&per_page=30&page=3"
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
    let secret_store = Arc::new(SecretStore::ephemeral());
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
                provider: VendorId::new("github").unwrap(),
                setup: ironclaw_host_api::capability::RuntimeCredentialAccountSetup::ManualToken,
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
            None,
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
    let secret_store = Arc::new(SecretStore::ephemeral());
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
                provider: VendorId::new("google").unwrap(),
                setup: ironclaw_host_api::capability::RuntimeCredentialAccountSetup::OAuth {
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
            None,
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
async fn host_runtime_services_extracts_google_drive_download_binary_into_text() {
    // Producer -> consumer boundary for `google-drive.download_file`: the
    // bundled WASM guest base64-encodes a binary download under `content_base64`
    // (it cannot run the host's document extractor), and the host completion /
    // obligation path must swap that base64 for extracted `content` before the
    // result reaches the model. Drives the FULL path through
    // `invoke_capability` (which routes through `complete_dispatch` ->
    // `extract_documents_in_output`), not the helper in isolation.
    let capability_id = CapabilityId::new("google-drive.download_file").unwrap();
    let scope = sample_scope(InvocationId::new());
    let policy = google_drive_policy();
    // `download_file` makes two HTTP calls: GET metadata (must parse as JSON)
    // then GET the media body (`?alt=media`). A PDF is binary (invalid UTF-8),
    // so the guest returns it base64-encoded for the host to extract.
    let pdf = include_bytes!("../../../../tests/fixtures/hello.pdf").to_vec();
    let network = SequencedGoogleDriveDownloadEgress::new(
        br#"{"id":"file-1","name":"hello.pdf","mimeType":"application/pdf"}"#.to_vec(),
        pdf,
    );
    let secret_store = Arc::new(SecretStore::ephemeral());
    let account_access_secret = SecretHandle::new("google_drive_download_access").unwrap();
    let required_scopes = vec!["https://www.googleapis.com/auth/drive.readonly".to_string()];
    let services = google_wasm_services_for_test!(
        "google-drive",
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
            None,
        )
        .await
        .unwrap();

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"file_id": "file-1"}),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id);
            assert!(
                completed.output.get("content_base64").is_none(),
                "host must strip `content_base64` so raw bytes never reach the model, got: {:?}",
                completed.output
            );
            let content = completed.output["content"].as_str().unwrap_or("");
            assert!(
                content.contains("Hello"),
                "extracted document text must be present, got: {content}"
            );
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
    // Two egress calls: metadata then media.
    let requests = network.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].url.contains("alt=media"));
}

#[tokio::test]
async fn host_runtime_services_maps_google_drive_wasm_401_to_auth_required() {
    let capability_id = CapabilityId::new("google-drive.list_files").unwrap();
    let scope = sample_scope(InvocationId::new());
    let policy = google_drive_policy();
    let network = RecordingNetworkHttpEgress::with_status_body(
        401,
        br#"{"error":{"status":"UNAUTHENTICATED","message":"Invalid Credentials"}}"#.to_vec(),
    );
    let secret_store = Arc::new(SecretStore::ephemeral());
    let account_access_secret = SecretHandle::new("google_drive_access").unwrap();
    let required_scopes = vec!["https://www.googleapis.com/auth/drive.readonly".to_string()];
    let services = google_wasm_services_for_test!(
        "google-drive",
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
            SecretMaterial::from("ya29.expired_fixture_token"),
            None,
        )
        .await
        .unwrap();

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({}),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::AuthRequired(gate) => {
            assert_eq!(gate.capability_id, capability_id);
            assert!(gate.required_secrets.is_empty());
            // The runtime 401 carries no auth detail of its own; the capability
            // host enriches the gate from the single credential obligation so the
            // WebUI can launch the google OAuth re-auth flow. An empty list here is
            // the provider-null, unsubmittable gate (#5174). Inline/background
            // refresh already ran before injection; a runtime 401 is the genuine
            // re-auth fallback, so the gate must surface provider + OAuth setup.
            assert_eq!(gate.credential_requirements.len(), 1);
            let requirement = &gate.credential_requirements[0];
            assert_eq!(requirement.provider, VendorId::new("google").unwrap());
            assert_eq!(
                requirement.setup,
                ironclaw_host_api::capability::RuntimeCredentialAccountSetup::OAuth {
                    scopes: vec!["https://www.googleapis.com/auth/drive.readonly".to_string()]
                }
            );
        }
        other => panic!("expected auth-required outcome, got {other:?}"),
    }
    let requests = network.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, NetworkMethod::Get);
    assert!(
        requests[0]
            .url
            .starts_with("https://www.googleapis.com/drive/v3/files?")
    );
}

#[tokio::test]
async fn host_runtime_services_maps_google_drive_upload_wasm_401_to_auth_required() {
    let capability_id = CapabilityId::new("google-drive.upload_file").unwrap();
    let scope = sample_scope(InvocationId::new());
    let policy = google_drive_policy();
    let network = RecordingNetworkHttpEgress::with_status_body(
        401,
        br#"{"error":{"status":"UNAUTHENTICATED","message":"Invalid Credentials"}}"#.to_vec(),
    );
    let secret_store = Arc::new(SecretStore::ephemeral());
    let account_access_secret = SecretHandle::new("google_drive_upload_access").unwrap();
    let required_scopes = vec!["https://www.googleapis.com/auth/drive".to_string()];
    let services = google_wasm_services_for_test!(
        "google-drive",
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
            SecretMaterial::from("ya29.expired_upload_fixture_token"),
            None,
        )
        .await
        .unwrap();

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({
                "name": "report.txt",
                "content": "stale token upload",
                "mime_type": "text/plain"
            }),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::AuthRequired(gate) => {
            assert_eq!(gate.capability_id, capability_id);
            assert!(gate.required_secrets.is_empty());
            // See list_files counterpart above: enrichment surfaces the single
            // credential obligation's provider + OAuth setup so the re-auth gate is
            // submittable (#5174). Empty would be the regressed provider-null gate.
            assert_eq!(gate.credential_requirements.len(), 1);
            let requirement = &gate.credential_requirements[0];
            assert_eq!(requirement.provider, VendorId::new("google").unwrap());
            assert_eq!(
                requirement.setup,
                ironclaw_host_api::capability::RuntimeCredentialAccountSetup::OAuth {
                    scopes: vec!["https://www.googleapis.com/auth/drive".to_string()]
                }
            );
        }
        other => panic!("expected auth-required outcome, got {other:?}"),
    }
    let requests = network.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, NetworkMethod::Post);
    assert!(
        requests[0]
            .url
            .starts_with("https://www.googleapis.com/upload/drive/v3/files?")
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
    let secret_store = Arc::new(SecretStore::ephemeral());
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
            None,
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
    let secret_store = Arc::new(SecretStore::ephemeral());
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
            None,
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
    let secret_store = Arc::new(SecretStore::ephemeral());
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
            None,
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
    let secret_store = Arc::new(SecretStore::ephemeral());
    let account_access_secret = SecretHandle::new("github_manual_access").unwrap();
    let services = github_wasm_services_for_test!(
        network.clone(),
        Arc::clone(&secret_store),
        account_access_secret.clone(),
    );
    secret_store
        .put(
            scope.clone(),
            account_access_secret,
            SecretMaterial::from("ghp_fake_fixture_token"),
            None,
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

    assert_failed_outcome(outcome, FailureKind::InputEncode);
    assert!(
        network.requests().is_empty(),
        "guest validation failures must block before HTTP egress"
    );
}

#[tokio::test]
async fn host_runtime_services_maps_github_search_validation_status_to_invalid_input() {
    let capability_id = CapabilityId::new("github.search_issues").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = RecordingNetworkHttpEgress::with_status_body(
        422,
        br#"{"message":"Validation Failed","errors":[{"message":"\"YYYY-MM-DD\" is not a recognized date/time format.","resource":"Search","field":"q","code":"invalid"}],"status":"422"}"#.to_vec(),
    );
    let secret_store = Arc::new(SecretStore::ephemeral());
    let account_access_secret = SecretHandle::new("github_manual_access").unwrap();
    let services = github_wasm_services_for_test!(
        network.clone(),
        Arc::clone(&secret_store),
        account_access_secret.clone(),
    );
    secret_store
        .put(
            scope.clone(),
            account_access_secret,
            SecretMaterial::from("ghp_fake_fixture_token"),
            None,
        )
        .await
        .unwrap();

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id,
            scope,
            json!({"query": "author:serrrfirat is:pr created:YYYY-MM-DD", "limit": 1}),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, FailureKind::InputEncode);
    assert_eq!(network.requests().len(), 1);
}

#[tokio::test]
async fn host_runtime_services_keeps_github_non_validation_422_as_operation_failed() {
    let capability_id = CapabilityId::new("github.search_issues").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = RecordingNetworkHttpEgress::with_status_body(
        422,
        br#"{"message":"Validation failed, or the endpoint has been spammed.","status":"422"}"#
            .to_vec(),
    );
    let secret_store = Arc::new(SecretStore::ephemeral());
    let account_access_secret = SecretHandle::new("github_manual_access").unwrap();
    let services = github_wasm_services_for_test!(
        network.clone(),
        Arc::clone(&secret_store),
        account_access_secret.clone(),
    );
    secret_store
        .put(
            scope.clone(),
            account_access_secret,
            SecretMaterial::from("ghp_fake_fixture_token"),
            None,
        )
        .await
        .unwrap();

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id,
            scope,
            json!({"query": "author:serrrfirat is:pr created:YYYY-MM-DD", "limit": 1}),
        ))
        .await
        .unwrap();

    assert_failed_outcome(outcome, FailureKind::OperationFailed);
    assert_eq!(network.requests().len(), 1);
}

#[tokio::test]
async fn host_runtime_services_missing_github_runtime_secret_blocks_on_auth() {
    let capability_id = CapabilityId::new("github.search_issues").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = RecordingNetworkHttpEgress::with_body(
        br#"{"total_count":0,"incomplete_results":false,"items":[]}"#.to_vec(),
    );
    let secret_store = Arc::new(SecretStore::ephemeral());
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
                provider: VendorId::new("github").unwrap(),
                setup: ironclaw_host_api::capability::RuntimeCredentialAccountSetup::ManualToken,
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

/// Audit F-010: per-user token isolation for the `slack_user` first-party
/// tool. A `slack_user` capability dispatch must inject the *authenticated
/// user's personal* Slack token — the `xoxp-` user token resolved from the
/// per-user `slack` product-auth account — as the
/// `Authorization: Bearer` header on the slack.com egress, and never the
/// workspace bot (`xoxb-`) token. This mirrors the github/google search
/// injection contracts above, driven through the full `invoke_capability`
/// path (authorizer obligation -> credential-account resolver -> secret
/// store -> staged injection -> recorded egress).
#[tokio::test]
async fn host_runtime_services_injects_personal_xoxp_token_for_slack_user_search_capability() {
    let capability_id = CapabilityId::new("slack.search_messages").unwrap();
    let scope = sample_scope(InvocationId::new());
    // A per-user *personal* Slack token, shaped like a real `xoxp-` user token
    // so a regression that swapped in a bot (`xoxb-`) token changes the bytes.
    let personal_user_token = "xoxp-fake-personal-user-token-9999";
    let policy = slack_policy();
    let network = RecordingNetworkHttpEgress::with_body(
        br#"{"ok":true,"messages":{"total":2,"matches":[]}}"#.to_vec(),
    );
    let secret_store = Arc::new(SecretStore::ephemeral());
    let slot_handle = SecretHandle::new("slack_user_token").unwrap();
    let account_access_secret = SecretHandle::new("slack_access").unwrap();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_slack_user_package()),
        Arc::new(filesystem_with_slack_user_package()),
        Arc::new(governor_with_default_limit(sample_account())),
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ApplyNetworkPolicy {
                policy: policy.clone(),
            },
            Obligation::InjectCredentialAccountOnce {
                handle: slot_handle,
                provider: VendorId::new("slack").unwrap(),
                setup: ironclaw_host_api::capability::RuntimeCredentialAccountSetup::OAuth {
                    scopes: slack_user_scopes(),
                },
                provider_scopes: slack_user_scopes(),
                requester_extension: ExtensionId::new("slack").unwrap(),
            },
        ])),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_secret_store(Arc::clone(&secret_store))
    .with_runtime_credential_account_resolver(Arc::new(
        FixedSlackRuntimeCredentialAccountResolver {
            expected_scopes: slack_user_scopes(),
            result: Ok(account_access_secret.clone()),
        },
    ))
    .with_trust_policy(Arc::new(slack_user_first_party_trust_policy()))
    .try_with_host_http_egress(network.clone())
    .unwrap()
    .try_with_wasm_runtime(WitToolRuntimeConfig::default(), WitToolHost::deny_all())
    .unwrap();
    secret_store
        .put(
            scope.clone(),
            account_access_secret,
            SecretMaterial::from(personal_user_token),
            None,
        )
        .await
        .unwrap();

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"query": "deploy"}),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => {
            assert_eq!(completed.capability_id, capability_id);
            assert_eq!(completed.output["matches"], json!([]));
            assert_eq!(completed.output["total"], json!(2));
        }
        other => panic!("expected completed outcome, got {other:?}"),
    }
    let requests = network.requests();
    // search_messages now also calls auth.test (best-effort) to mark each
    // match's canonical `is_self`; the search call itself still goes out
    // first, matches unscripted here.
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, NetworkMethod::Get);
    assert_eq!(
        requests[0].url,
        "https://slack.com/api/search.messages?query=deploy&count=20"
    );
    assert_eq!(requests[0].policy, policy);
    // The injected credential is the per-user personal xoxp token that was
    // resolved from the `slack` account and stored under this scope — proven
    // for BOTH requests this dispatch makes, never a workspace bot token:
    // F-010 requires the personal (`xoxp-`) user token, not the `xoxb-` bot
    // token the `slack` channel uses.
    for request in &requests {
        let authorization = request
            .headers
            .iter()
            .find(|(name, _)| name == "authorization");
        assert_eq!(
            authorization,
            Some(&(
                "authorization".to_string(),
                format!("Bearer {personal_user_token}"),
            )),
            "{}: expected the personal token injected",
            request.url
        );
        let (_, header_value) = authorization.unwrap();
        assert!(
            !header_value.contains("xoxb-"),
            "slack_user must not inject a bot token, got: {header_value}"
        );
    }
}

/// Audit F-010 companion: a MISSING `slack` account must gate the
/// `slack_user` tool on auth — it must never silently fall back to another
/// credential (e.g. the workspace bot token). Mirrors the github
/// missing-secret contract: the resolver returns `AuthRequired`, and no
/// slack.com egress happens.
#[tokio::test]
async fn host_runtime_services_missing_slack_account_blocks_slack_user_on_auth() {
    let capability_id = CapabilityId::new("slack.search_messages").unwrap();
    let scope = sample_scope(InvocationId::new());
    let policy = slack_policy();
    let network = RecordingNetworkHttpEgress::with_body(
        br#"{"ok":true,"messages":{"total":0,"matches":[]}}"#.to_vec(),
    );
    let secret_store = Arc::new(SecretStore::ephemeral());
    let slot_handle = SecretHandle::new("slack_user_token").unwrap();
    let services = HostRuntimeServices::new(
        Arc::new(registry_with_slack_user_package()),
        Arc::new(filesystem_with_slack_user_package()),
        Arc::new(governor_with_default_limit(sample_account())),
        Arc::new(ObligatingAuthorizer::new(vec![
            Obligation::ApplyNetworkPolicy {
                policy: policy.clone(),
            },
            Obligation::InjectCredentialAccountOnce {
                handle: slot_handle,
                provider: VendorId::new("slack").unwrap(),
                setup: ironclaw_host_api::capability::RuntimeCredentialAccountSetup::OAuth {
                    scopes: slack_user_scopes(),
                },
                provider_scopes: slack_user_scopes(),
                requester_extension: ExtensionId::new("slack").unwrap(),
            },
        ])),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_secret_store(Arc::clone(&secret_store))
    .with_runtime_credential_account_resolver(Arc::new(
        FixedSlackRuntimeCredentialAccountResolver {
            expected_scopes: slack_user_scopes(),
            result: Err(CredentialStageError::AuthRequired),
        },
    ))
    .with_trust_policy(Arc::new(slack_user_first_party_trust_policy()))
    .try_with_host_http_egress(network.clone())
    .unwrap()
    .try_with_wasm_runtime(WitToolRuntimeConfig::default(), WitToolHost::deny_all())
    .unwrap();

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"query": "deploy"}),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::AuthRequired(gate) => {
            assert_eq!(gate.capability_id, capability_id);
        }
        other => panic!("expected auth-required outcome, got {other:?}"),
    }
    assert!(
        network.requests().is_empty(),
        "missing slack account must block before slack.com egress"
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
    let search_output: serde_json::Value = serde_json::from_str(
        search
            .output_json
            .as_deref()
            .expect("search should return compact JSON"),
    )
    .expect("compact search output should be valid JSON");
    assert_eq!(
        search_output,
        json!({"total_count": 0, "incomplete_results": false, "items": []})
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

    let list_my_repos_http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 200,
        headers_json: "{}".to_string(),
        body: br#"[]"#.to_vec(),
    }));
    let list_my_repos = execute_bundled_github_wasm(
        "github.list_repos",
        json!({"limit": 2}),
        Arc::clone(&list_my_repos_http),
    );
    assert_eq!(list_my_repos.error, None);
    assert_single_wasm_request(
        &list_my_repos_http,
        "GET",
        "https://api.github.com/user/repos?per_page=2",
        None,
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
async fn bundled_github_wasm_get_authenticated_user_uses_user_endpoint() {
    let http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 200,
        headers_json: "{}".to_string(),
        body: br#"{"login":"serrrfirat","type":"User"}"#.to_vec(),
    }));
    let user = execute_bundled_github_wasm(
        "github.get_authenticated_user",
        json!({}),
        Arc::clone(&http),
    );

    assert_eq!(user.error, None);
    let user: serde_json::Value =
        serde_json::from_str(user.output_json.as_deref().unwrap()).unwrap();
    assert_eq!(user["login"], json!("serrrfirat"));
    assert_eq!(user["type"], json!("User"));
    assert_single_wasm_request(&http, "GET", "https://api.github.com/user", None);
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
        "github.get_issue",
        json!({"owner": "nearai", "repo": "ironclaw", "issue_number": 1}),
        Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
            status: 200,
            headers_json: "{}".to_string(),
            body: b"not-json".to_vec(),
        })),
    );

    assert_eq!(execution.output_json.as_deref(), Some("not-json"));
    assert_eq!(execution.error, None);
}

#[tokio::test]
async fn bundled_github_wasm_rejects_malformed_compacted_search_response() {
    let execution = execute_bundled_github_wasm(
        "github.search_issues",
        json!({"query": "repo:nearai/ironclaw is:issue", "limit": 1}),
        Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
            status: 200,
            headers_json: "{}".to_string(),
            body: b"not-json".to_vec(),
        })),
    );

    assert_eq!(execution.output_json, None);
    assert_eq!(
        structured_wasm_error_code(&execution).as_deref(),
        Some("github_api_invalid_response")
    );
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

fn assert_failed_outcome(outcome: RuntimeCapabilityOutcome, expected_kind: FailureKind) {
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
    status: u16,
    response_body: Vec<u8>,
}

impl RecordingNetworkHttpEgress {
    fn with_body(response_body: Vec<u8>) -> Self {
        Self::with_status_body(200, response_body)
    }

    fn with_status_body(status: u16, response_body: Vec<u8>) -> Self {
        Self {
            requests: Arc::new(std::sync::Mutex::new(Vec::new())),
            status,
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
            status: self.status,
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

/// Network egress stub for the `download_file` two-call flow: the Drive guest
/// first GETs file metadata (JSON) and then GETs the media body (`?alt=media`).
/// A single fixed-body egress can't serve both, so this returns the metadata
/// JSON for the metadata request and the binary file bytes for the media
/// request (discriminated on `alt=media` in the URL).
#[derive(Debug, Clone)]
struct SequencedGoogleDriveDownloadEgress {
    requests: Arc<std::sync::Mutex<Vec<NetworkHttpRequest>>>,
    metadata_body: Vec<u8>,
    media_body: Vec<u8>,
}

impl SequencedGoogleDriveDownloadEgress {
    fn new(metadata_body: Vec<u8>, media_body: Vec<u8>) -> Self {
        Self {
            requests: Arc::new(std::sync::Mutex::new(Vec::new())),
            metadata_body,
            media_body,
        }
    }

    fn requests(&self) -> Vec<NetworkHttpRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl NetworkHttpEgress for SequencedGoogleDriveDownloadEgress {
    async fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let request_bytes = request.body.len() as u64;
        let is_media = request.url.contains("alt=media");
        self.requests.lock().unwrap().push(request);
        let body = if is_media {
            self.media_body.clone()
        } else {
            self.metadata_body.clone()
        };
        let response_bytes = body.len() as u64;
        Ok(NetworkHttpResponse {
            status: 200,
            headers: Vec::new(),
            body,
            usage: NetworkUsage {
                request_bytes,
                response_bytes,
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

// ---- slack_user (audit F-010) helpers ----------------------------------
//
// Mirror the github/google first-party WASM harness above for the `slack_user`
// tool: load its bundled manifest + WASM, drive a capability through
// `invoke_capability`, and assert the per-user personal-token injection.

/// Credential-account resolver for the `slack_user` tool. Asserts the runtime
/// asks for the per-user *personal* account (`slack`) on behalf of the
/// `slack_user` extension — never a workspace/bot credential — before handing
/// back the fixed access-secret handle (or an auth-required error).
#[derive(Debug)]
struct FixedSlackRuntimeCredentialAccountResolver {
    expected_scopes: Vec<String>,
    result: Result<SecretHandle, CredentialStageError>,
}

#[async_trait]
impl RuntimeCredentialAccountResolver for FixedSlackRuntimeCredentialAccountResolver {
    async fn resolve_access_secret(
        &self,
        request: RuntimeCredentialAccountRequest<'_>,
    ) -> Result<RuntimeCredentialAccessSecret, CredentialStageError> {
        assert_eq!(request.provider.as_str(), "slack");
        assert_eq!(request.requester_extension.as_str(), "slack");
        assert_eq!(request.provider_scopes, self.expected_scopes.as_slice());
        self.result
            .clone()
            .map(|handle| RuntimeCredentialAccessSecret {
                scope: request.scope.clone(),
                handle,
            })
    }
}

fn registry_with_slack_user_package() -> ExtensionRegistry {
    // Parse through the single record entry point (the bundled asset is a
    // manifest v3 document).
    let root = VirtualPath::new("/system/extensions/slack").unwrap();
    let record = ironclaw_extension_registry::ExtensionManifestRecord::from_toml(
        std::fs::read_to_string(slack_user_asset_root().join("manifest.toml")).unwrap(),
        ManifestSource::HostBundled,
        &default_host_port_catalog().unwrap(),
        None,
        &default_host_api_contract_registry().unwrap(),
        Some(root.clone()),
    )
    .unwrap();
    let manifest = ExtensionManifest::try_from(record.manifest().clone()).unwrap();
    let package = ExtensionPackage::from_manifest(manifest, root).unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
}

fn filesystem_with_slack_user_package() -> DiskFilesystem {
    let mut filesystem = DiskFilesystem::new();
    filesystem
        .mount_local(
            VirtualPath::new("/system/extensions").unwrap(),
            HostPath::from_path_buf(slack_user_asset_root().parent().unwrap().to_path_buf()),
        )
        .unwrap();
    filesystem
}

/// The nearest ancestor holding both `crates/` and `Cargo.toml`.
///
/// A search, not counted `..` hops: the family move (PROPOSAL §5) makes any
/// fixed depth false, and the wrong answer is `crates/` — a directory that
/// exists, so the failure is a confusing missing-asset rather than a clear
/// one (CHECKLIST WS10).
fn workspace_root() -> std::path::PathBuf {
    let mut current = Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")));
    while let Some(directory) = current {
        if directory.join("crates").is_dir() && directory.join("Cargo.toml").is_file() {
            return directory.to_path_buf();
        }
        current = directory.parent();
    }
    panic!("no workspace root above {}", env!("CARGO_MANIFEST_DIR"));
}

fn slack_user_asset_root() -> std::path::PathBuf {
    workspace_root().join("crates/extensions/packages/slack")
}

fn slack_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "slack.com".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(10_000),
    }
}

/// The read-only scopes the Slack read capabilities (e.g. slack.search_messages)
/// request. Kept in lockstep with `crates/extensions/packages/slack/manifest.toml`, where the
/// read-only tools request only read scopes and only send_message adds chat:write.
fn slack_user_scopes() -> Vec<String> {
    [
        "search:read",
        "channels:history",
        "groups:history",
        "im:history",
        "mpim:history",
        "channels:read",
        "groups:read",
        "im:read",
        "mpim:read",
        "users:read",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

/// The scope set `slack.send_message` requests: the read scopes plus
/// chat:write, in lockstep with its manifest runtime_credentials entry.
/// `edit_message` and `delete_message` bind the same set — `chat.update` and
/// `chat.delete` ride the existing chat:write grant.
fn slack_user_write_scopes() -> Vec<String> {
    let mut scopes = slack_user_scopes();
    scopes.push("chat:write".to_string());
    scopes
}

/// The read scopes plus one tool's write-family additions, in lockstep with
/// that tool's manifest `[[tools.credentials]]` entry: `add_reaction` adds
/// reactions:write alone, `remove_reaction` adds reactions:read +
/// reactions:write (the omit-emoji variant reads the message's reactions
/// first), and `open_dm` adds im:write.
fn slack_user_scopes_plus(extra: &[&str]) -> Vec<String> {
    let mut scopes = slack_user_scopes();
    scopes.extend(extra.iter().map(|scope| (*scope).to_string()));
    scopes
}

fn slack_user_first_party_trust_policy() -> HostTrustPolicy {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("slack").unwrap(),
            "/system/extensions/slack/manifest.toml".to_string(),
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

fn registry_with_github_package() -> ExtensionRegistry {
    // Parse through the single record entry point (the bundled asset is a
    // manifest v3 document).
    let root = VirtualPath::new("/system/extensions/github").unwrap();
    let record = ironclaw_extension_registry::ExtensionManifestRecord::from_toml(
        std::fs::read_to_string(github_asset_root().join("manifest.toml")).unwrap(),
        ManifestSource::HostBundled,
        &default_host_port_catalog().unwrap(),
        None,
        &default_host_api_contract_registry().unwrap(),
        Some(root.clone()),
    )
    .unwrap();
    let manifest = ExtensionManifest::try_from(record.manifest().clone()).unwrap();
    let package = ExtensionPackage::from_manifest(manifest, root).unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
}

fn registry_with_google_drive_package() -> ExtensionRegistry {
    registry_with_google_package("google-drive")
}

fn filesystem_with_github_package() -> DiskFilesystem {
    let mut filesystem = DiskFilesystem::new();
    filesystem
        .mount_local(
            VirtualPath::new("/system/extensions").unwrap(),
            HostPath::from_path_buf(github_asset_root().parent().unwrap().to_path_buf()),
        )
        .unwrap();
    filesystem
}

fn filesystem_with_google_drive_package() -> DiskFilesystem {
    filesystem_with_google_package("google-drive")
}

fn registry_with_google_package(package_id: &str) -> ExtensionRegistry {
    // Parse through the single record entry point (the bundled asset is a
    // manifest v3 document).
    let root = VirtualPath::new(format!("/system/extensions/{package_id}")).unwrap();
    let record = ironclaw_extension_registry::ExtensionManifestRecord::from_toml(
        std::fs::read_to_string(google_asset_root(package_id).join("manifest.toml")).unwrap(),
        ManifestSource::HostBundled,
        &default_host_port_catalog().unwrap(),
        None,
        &default_host_api_contract_registry().unwrap(),
        Some(root.clone()),
    )
    .unwrap();
    let manifest = ExtensionManifest::try_from(record.manifest().clone()).unwrap();
    let package = ExtensionPackage::from_manifest(manifest, root).unwrap();
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
}

fn filesystem_with_google_package(package_id: &str) -> DiskFilesystem {
    let mut filesystem = DiskFilesystem::new();
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
    workspace_root().join("crates/extensions/packages/github")
}

fn google_drive_asset_root() -> std::path::PathBuf {
    google_asset_root("google-drive")
}

fn google_asset_root(package_id: &str) -> std::path::PathBuf {
    workspace_root()
        .join("crates/extensions/packages")
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
) -> RuntimeInvocation {
    let context = execution_context_with_dispatch_grant_for_scope(capability_id.clone(), scope);
    (context, capability_id, wasm_http_estimate(), input)
}

fn execution_context_with_dispatch_grant_for_scope(
    capability: CapabilityId,
    scope: ResourceScope,
) -> ExecutionContext {
    let context = ExecutionContext {
        run_id: Some(RunId::new()),
        origin: None,
        invocation_id: scope.invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        authenticated_actor_user_id: None,
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

fn execute_bundled_github_wasm(
    capability_id: &str,
    input: serde_json::Value,
    http: Arc<RecordingWasmHostHttp>,
) -> WitToolExecution {
    let (runtime, prepared) = bundled_github_runtime();
    runtime
        .execute(
            prepared,
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
    let (runtime, prepared) = bundled_google_drive_runtime();
    let request = match context {
        Some(context) => WitToolRequest::new(input.to_string()).with_context(context.to_string()),
        None => WitToolRequest::new(input.to_string()),
    };
    runtime
        .execute(prepared, WitToolHost::deny_all().with_http(http), request)
        .unwrap()
}

fn bundled_github_runtime() -> &'static (WitToolRuntime, PreparedWitTool) {
    static BUNDLE: OnceLock<(WitToolRuntime, PreparedWitTool)> = OnceLock::new();
    BUNDLE.get_or_init(|| {
        let runtime = WitToolRuntime::new(WitToolRuntimeConfig::default()).unwrap();
        let wasm_bytes =
            std::fs::read(github_wasm_path()).expect("first-party GitHub WASM must be built");
        let prepared = runtime.prepare("github", &wasm_bytes).unwrap();
        (runtime, prepared)
    })
}

fn bundled_google_drive_runtime() -> &'static (WitToolRuntime, PreparedWitTool) {
    static BUNDLE: OnceLock<(WitToolRuntime, PreparedWitTool)> = OnceLock::new();
    BUNDLE.get_or_init(|| {
        let runtime = WitToolRuntime::new(WitToolRuntimeConfig::default()).unwrap();
        let wasm_bytes = std::fs::read(google_drive_wasm_path())
            .expect("first-party Google Drive WASM must be built");
        let prepared = runtime.prepare("google-drive", &wasm_bytes).unwrap();
        (runtime, prepared)
    })
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
            ResourceLimits::default()
                .set_max_concurrency_slots(10)
                .set_max_network_egress_bytes(10_000)
                .set_max_output_bytes(100_000),
        )
        .unwrap();
    governor
}

fn wasm_http_estimate() -> ResourceEstimate {
    ResourceEstimate::default()
        .set_concurrency_slots(1)
        .set_network_egress_bytes(10)
        .set_output_bytes(10_000)
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

// ─── Slack personal (user-token) WASM tool ──────────────────────────────────
//
// Same contract tier as the GitHub/Google tests above: the REAL bundled
// `slack_user_tool.wasm` dispatched through the full `invoke_capability`
// path, with the network scripted per Slack Web API method. Pins the
// ID→name enrichment contract: read outputs carry human-readable
// `author.display_name` fields alongside raw Slack user ids, resolved inside
// the tool (one `users.info` per distinct id), and enrichment is
// best-effort — a failing `users.info` must never break the read itself.

/// URL-substring-keyed scripted egress: first matching entry serves the
/// response. The Slack enrichment flow needs different bodies for
/// `conversations.history` / `conversations.list` and each `users.info`
/// lookup, which the single fixed-body recorder above cannot express.
#[derive(Debug, Clone)]
struct UrlKeyedSlackEgress {
    requests: Arc<std::sync::Mutex<Vec<NetworkHttpRequest>>>,
    responses: Vec<(&'static str, u16, &'static str)>,
}

impl UrlKeyedSlackEgress {
    fn new(responses: Vec<(&'static str, u16, &'static str)>) -> Self {
        Self {
            requests: Arc::new(std::sync::Mutex::new(Vec::new())),
            responses,
        }
    }

    fn requests(&self) -> Vec<NetworkHttpRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl NetworkHttpEgress for UrlKeyedSlackEgress {
    async fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let request_bytes = request.body.len() as u64;
        let matched = self
            .responses
            .iter()
            .find(|(needle, _, _)| request.url.contains(needle))
            .map(|(_, status, body)| (*status, body.as_bytes().to_vec()))
            .unwrap_or((404, b"{\"ok\":false,\"error\":\"unscripted_url\"}".to_vec()));
        self.requests.lock().unwrap().push(request);
        Ok(NetworkHttpResponse {
            status: matched.0,
            headers: Vec::new(),
            body: matched.1.clone(),
            usage: NetworkUsage {
                request_bytes,
                response_bytes: matched.1.len() as u64,
                resolved_ip: None,
            },
        })
    }
}

/// Ordered scripted egress: serves responses strictly first-in-first-out,
/// one per request, regardless of URL (404 when exhausted). Needed where the
/// SAME endpoint must answer differently across sequential calls — e.g. the
/// `chat.postMessage` as_user retry, which `UrlKeyedSlackEgress` cannot
/// express because its first matching entry always wins.
#[derive(Debug, Clone)]
struct SequencedSlackEgress {
    requests: Arc<std::sync::Mutex<Vec<NetworkHttpRequest>>>,
    responses: Arc<std::sync::Mutex<std::collections::VecDeque<(u16, &'static str)>>>,
}

impl SequencedSlackEgress {
    fn new(responses: Vec<(u16, &'static str)>) -> Self {
        Self {
            requests: Arc::new(std::sync::Mutex::new(Vec::new())),
            responses: Arc::new(std::sync::Mutex::new(responses.into_iter().collect())),
        }
    }

    fn requests(&self) -> Vec<NetworkHttpRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl NetworkHttpEgress for SequencedSlackEgress {
    async fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let request_bytes = request.body.len() as u64;
        let matched = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .map(|(status, body)| (status, body.as_bytes().to_vec()))
            .unwrap_or((
                404,
                b"{\"ok\":false,\"error\":\"unscripted_call\"}".to_vec(),
            ));
        self.requests.lock().unwrap().push(request);
        Ok(NetworkHttpResponse {
            status: matched.0,
            headers: Vec::new(),
            body: matched.1.clone(),
            usage: NetworkUsage {
                request_bytes,
                response_bytes: matched.1.len() as u64,
                resolved_ip: None,
            },
        })
    }
}

/// Build the Slack services + seed the personal token for the enrichment
/// contract tests, reusing the F-010 fixtures (`registry_with_slack_user_package`
/// / `slack_policy` / `slack_user_scopes` / trust policy) with the URL-keyed
/// egress above.
macro_rules! slack_enrichment_services_for_test {
    ($network:expr, $secret_store:expr $(,)?) => {
        slack_enrichment_services_for_test!($network, $secret_store, slack_user_scopes())
    };
    ($network:expr, $secret_store:expr, $scopes:expr $(,)?) => {{
        HostRuntimeServices::new(
            Arc::new(registry_with_slack_user_package()),
            Arc::new(filesystem_with_slack_user_package()),
            Arc::new(governor_with_default_limit(sample_account())),
            Arc::new(ObligatingAuthorizer::new(vec![
                Obligation::ApplyNetworkPolicy {
                    policy: slack_policy(),
                },
                Obligation::InjectCredentialAccountOnce {
                    handle: SecretHandle::new("slack_user_token").unwrap(),
                    provider: VendorId::new("slack").unwrap(),
                    setup: ironclaw_host_api::capability::RuntimeCredentialAccountSetup::OAuth {
                        scopes: $scopes,
                    },
                    provider_scopes: $scopes,
                    requester_extension: ExtensionId::new("slack").unwrap(),
                },
            ])),
            ProcessServices::in_memory(),
            CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        )
        .with_secret_store($secret_store)
        .with_runtime_credential_account_resolver(Arc::new(
            FixedSlackRuntimeCredentialAccountResolver {
                expected_scopes: $scopes,
                result: Ok(SecretHandle::new("slack_personal_access").unwrap()),
            },
        ))
        .with_trust_policy(Arc::new(slack_user_first_party_trust_policy()))
        .try_with_host_http_egress($network)
        .unwrap()
        .try_with_wasm_runtime(WitToolRuntimeConfig::default(), WitToolHost::deny_all())
        .unwrap()
    }};
}

async fn seed_slack_user_token(secret_store: &SecretStore<InMemoryBackend>, scope: &ResourceScope) {
    secret_store
        .put(
            scope.clone(),
            SecretHandle::new("slack_personal_access").unwrap(),
            SecretMaterial::from("xoxp-fake-fixture-token"),
            None,
        )
        .await
        .unwrap();
}

const SLACK_HISTORY_BODY: &str = r#"{"ok":true,"has_more":false,"messages":[
    {"type":"message","user":"U0AAA","text":"hey","ts":"1751970001.000100","thread_ts":"1751970001.000100","reply_count":2,"edited":{"user":"U0AAA","ts":"1751970010.000100"}},
    {"type":"message","user":"U0BBB","text":"yo","ts":"1751970002.000100"},
    {"type":"message","user":"U0AAA","text":"again","ts":"1751970003.000100"}
]}"#;
const SLACK_USER_AAA_BODY: &str = r#"{"ok":true,"user":{"id":"U0AAA","name":"firat","is_bot":false,"profile":{"display_name":"Firat","real_name":"Firat Sertgoz"}}}"#;
const SLACK_USER_BBB_BODY: &str = r#"{"ok":true,"user":{"id":"U0BBB","name":"ada","is_bot":false,"profile":{"display_name":"","real_name":"Ada Lovelace"}}}"#;
const SLACK_AUTH_TEST_SELF_AAA_BODY: &str =
    r#"{"ok":true,"user_id":"U0AAA","user":"firat","team_id":"T0TEAM"}"#;

/// Digest bug regression (raw `U0ANBHZUUUR`-style ids in user-facing output):
/// history messages must carry `author.display_name` resolved INSIDE the
/// tool — one `users.info` call per DISTINCT author — so name resolution is
/// the default path, not something the model has to remember to do.
#[tokio::test]
async fn slack_history_output_carries_display_names_alongside_raw_user_ids() {
    let capability_id = CapabilityId::new("slack.get_conversation_history").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![
        ("conversations.history", 200, SLACK_HISTORY_BODY),
        ("users.info?user=U0AAA", 200, SLACK_USER_AAA_BODY),
        ("users.info?user=U0BBB", 200, SLACK_USER_BBB_BODY),
        ("auth.test", 200, SLACK_AUTH_TEST_SELF_AAA_BODY),
    ]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(network.clone(), Arc::clone(&secret_store));
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"conversation": "D0FIRAT"}),
        ))
        .await
        .unwrap();

    let output = match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => completed.output,
        other => panic!("expected completed outcome, got {other:?}"),
    };
    let messages = output["messages"].as_array().expect("messages array");
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["author"]["user_ref"], json!("U0AAA"));
    assert_eq!(
        messages[0]["author"]["display_name"],
        json!("Firat"),
        "history output must resolve raw user ids to display names: {output}"
    );
    assert_eq!(
        messages[1]["author"]["display_name"],
        json!("Ada Lovelace"),
        "empty display_name must fall back to real_name: {output}"
    );
    assert_eq!(messages[2]["author"]["display_name"], json!("Firat"));

    // One users.info per DISTINCT author — the in-call cache, not per-message.
    let lookups = network
        .requests()
        .iter()
        .filter(|request| request.url.contains("users.info"))
        .count();
    assert_eq!(
        lookups, 2,
        "expected exactly one users.info lookup per distinct author"
    );

    // Identity-attribution regression ("says George is off but it's actually
    // me"): the tool must mark which messages the CONNECTED account authored,
    // so the model can attribute the requester's own words to the requester.
    // The canonical output carries no top-level connected-account id (only
    // per-message `is_self`); the marking itself is the regression pin.
    assert_eq!(
        messages[0]["is_self"],
        json!(true),
        "messages authored by the connected account must be marked: {output}"
    );
    assert_eq!(
        messages[1]["is_self"],
        json!(false),
        "other authors must be explicitly not self: {output}"
    );
    assert_eq!(messages[2]["is_self"], json!(true));
    assert_eq!(
        messages[0]["edited"],
        json!(true),
        "Slack's edited object must become canonical edited=true: {output}"
    );

    // Threads: history returns only thread PARENTS (replies live behind
    // conversations.replies), so the parent must advertise its reply_count —
    // the model's cue to fetch the thread with slack.get_thread_replies.
    assert_eq!(
        messages[0]["thread"]["reply_count"],
        json!(2),
        "thread parents must surface reply_count: {output}"
    );
    assert!(
        messages[1].get("thread").is_none(),
        "non-parents must not fabricate a thread anchor: {output}"
    );

    // W13 (pre-merge amendment wave, verified defect): Slack's `ts` carries
    // real sub-second precision ("1751970001.000100" here is 100
    // microseconds past the whole second) — the canonical `timestamp` must
    // preserve it, not truncate to whole seconds.
    assert_eq!(
        messages[0]["timestamp"],
        json!("2025-07-08T10:20:01.000100Z"),
        "Slack ts fractional precision must survive into the RFC3339 timestamp: {output}"
    );
}

const SLACK_REPLIES_BODY: &str = r#"{"ok":true,"has_more":false,"messages":[
    {"type":"message","user":"U0AAA","text":"parent","ts":"1751970001.000100","thread_ts":"1751970001.000100","reply_count":2},
    {"type":"message","user":"U0BBB","text":"first reply <@U0AAA> &amp; co","ts":"1751970005.000100","thread_ts":"1751970001.000100"},
    {"type":"message","user":"U0AAA","text":"second reply","ts":"1751970006.000100","thread_ts":"1751970001.000100"}
]}"#;

/// Thread replies are NOT in `conversations.history`; `slack.get_thread_replies`
/// must fetch them via `conversations.replies` with the same enrichment
/// contract as history: resolved display names (one users.info per distinct
/// author) and connected-account marking.
#[tokio::test]
async fn slack_thread_replies_resolve_names_and_mark_connected_account() {
    let capability_id = CapabilityId::new("slack.get_thread_replies").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![
        ("conversations.replies", 200, SLACK_REPLIES_BODY),
        ("users.info?user=U0AAA", 200, SLACK_USER_AAA_BODY),
        ("users.info?user=U0BBB", 200, SLACK_USER_BBB_BODY),
        ("auth.test", 200, SLACK_AUTH_TEST_SELF_AAA_BODY),
    ]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(network.clone(), Arc::clone(&secret_store));
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"conversation": "C0GENERAL", "thread": "1751970001.000100"}),
        ))
        .await
        .unwrap();

    let output = match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => completed.output,
        other => panic!("expected completed outcome, got {other:?}"),
    };
    let replies_request = network
        .requests()
        .into_iter()
        .find(|request| request.url.contains("conversations.replies"))
        .expect("get_thread_replies must call conversations.replies");
    assert!(
        replies_request.url.contains("channel=C0GENERAL")
            && replies_request.url.contains("ts=1751970001.000100"),
        "conversations.replies must be keyed by channel + thread ts: {}",
        replies_request.url
    );

    let messages = output["messages"].as_array().expect("messages array");
    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages[1]["author"]["display_name"],
        json!("Ada Lovelace"),
        "thread replies must resolve display names like history does: {output}"
    );
    assert_eq!(
        messages[1]["text"],
        json!("first reply @Firat & co"),
        "thread reply text must resolve in-text mentions and entities: {output}"
    );
    assert_eq!(
        messages[0]["is_self"],
        json!(true),
        "thread replies must surface the connected account: {output}"
    );
    assert_eq!(
        messages[1]["is_self"],
        json!(false),
        "thread replies must mark the connected account's messages: {output}"
    );
}

/// DM entries from `list_conversations` carry the counterpart's display name
/// next to the raw `user` id (a DM has no `name` in Slack's API, so without
/// this the model can only echo the raw id).
#[tokio::test]
async fn slack_list_conversations_dms_carry_counterpart_display_names() {
    let capability_id = CapabilityId::new("slack.list_conversations").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![
        (
            "conversations.list",
            200,
            r#"{"ok":true,"channels":[{"id":"D0FIRAT","is_im":true,"is_channel":false,"is_private":false,"is_mpim":false,"user":"U0AAA"}]}"#,
        ),
        ("users.info?user=U0AAA", 200, SLACK_USER_AAA_BODY),
    ]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(network.clone(), Arc::clone(&secret_store));
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"kinds": ["dm"]}),
        ))
        .await
        .unwrap();

    let output = match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => completed.output,
        other => panic!("expected completed outcome, got {other:?}"),
    };
    let conversations = output["conversations"].as_array().expect("conversations");
    assert_eq!(conversations[0]["counterpart"]["user_ref"], json!("U0AAA"));
    assert_eq!(
        conversations[0]["counterpart"]["display_name"],
        json!("Firat"),
        "DM list entries must resolve the counterpart id to a display name: {output}"
    );
}

/// `conversations.list` reality check: Slack returns channels you can SEE,
/// not only ones you're in. The tool must surface `is_member` per channel
/// (absent for DMs, which have no membership axis), pass an input `cursor`
/// through, and return `next_cursor` so the model can page instead of
/// concluding from the first page.
#[tokio::test]
async fn slack_list_conversations_surfaces_membership_and_pagination() {
    let capability_id = CapabilityId::new("slack.list_conversations").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![(
        "conversations.list",
        200,
        r#"{"ok":true,"channels":[
            {"id":"C0IN","name":"general","is_channel":true,"is_private":false,"is_im":false,"is_mpim":false,"is_member":true},
            {"id":"C0OUT","name":"random","is_channel":true,"is_private":false,"is_im":false,"is_mpim":false,"is_member":false},
            {"id":"D0FIRAT","is_im":true,"is_channel":false,"is_private":false,"is_mpim":false,"user":"U0AAA"}
        ],"response_metadata":{"next_cursor":"dXNlcjpVMEc5V0ZYTlo="}}"#,
    )]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(network.clone(), Arc::clone(&secret_store));
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"kinds": ["channel", "dm"], "cursor": "page-two-cursor"}),
        ))
        .await
        .unwrap();

    let output = match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => completed.output,
        other => panic!("expected completed outcome, got {other:?}"),
    };
    let list_request = network
        .requests()
        .into_iter()
        .find(|request| request.url.contains("conversations.list"))
        .expect("list_conversations must call conversations.list");
    assert!(
        list_request.url.contains("cursor=page-two-cursor"),
        "input cursor must be passed through to Slack: {}",
        list_request.url
    );

    let conversations = output["conversations"].as_array().expect("conversations");
    assert_eq!(conversations[0]["is_member"], json!(true));
    assert_eq!(
        conversations[1]["is_member"],
        json!(false),
        "channels merely visible to the token must be marked non-member: {output}"
    );
    assert!(
        conversations[2].get("is_member").is_none(),
        "DMs have no membership axis; is_member must be absent, not fabricated: {output}"
    );
    assert_eq!(
        output["next_cursor"],
        json!("dXNlcjpVMEc5V0ZYTlo="),
        "next_cursor must surface so the model can page: {output}"
    );
}

/// QA-10F exact-target regression: when the prompt already supplies a DM
/// conversation ID, the capability must query that ID directly instead of
/// scanning a potentially truncated conversation list and guessing among
/// same-name users. Dispatch the real bundled WASM through HostRuntime so the
/// capability-id routing, HTTP request, response mapping, and name enrichment
/// are all covered together.
#[tokio::test]
async fn slack_get_conversation_info_resolves_exact_dm_counterpart() {
    let capability_id = CapabilityId::new("slack.get_conversation_info").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![
        (
            "conversations.info?channel=D0FIRAT",
            200,
            r#"{"ok":true,"channel":{"id":"D0FIRAT","is_channel":false,"is_private":true,"is_im":true,"is_mpim":false,"user":"U0BBB"}}"#,
        ),
        ("users.info?user=U0BBB", 200, SLACK_USER_BBB_BODY),
    ]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(network.clone(), Arc::clone(&secret_store));
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id,
            scope,
            json!({"conversation": "D0FIRAT"}),
        ))
        .await
        .unwrap();

    let output = match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => completed.output,
        other => panic!("expected completed outcome, got {other:?}"),
    };
    assert_eq!(output["conversation"], json!("D0FIRAT"));
    assert_eq!(output["counterpart"]["user_ref"], json!("U0BBB"));
    assert_eq!(output["counterpart"]["display_name"], json!("Ada Lovelace"));

    let requests = network.requests();
    assert!(
        requests
            .iter()
            .any(|request| request.url.ends_with("conversations.info?channel=D0FIRAT")),
        "exact conversation lookup was not sent: {requests:#?}"
    );
}

/// A successful HTTP response without the requested conversation must not
/// become a successful empty lookup that could send a later write elsewhere.
#[tokio::test]
async fn slack_get_conversation_info_rejects_missing_conversation_identity() {
    let capability_id = CapabilityId::new("slack.get_conversation_info").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![(
        "conversations.info?channel=D0FIRAT",
        200,
        r#"{"ok":true,"channel":{"is_im":true,"user":"U0BBB"}}"#,
    )]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(network, Arc::clone(&secret_store));
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id,
            scope,
            json!({"conversation": "D0FIRAT"}),
        ))
        .await
        .unwrap();

    let failure = match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => failure,
        other => panic!("expected failed outcome, got {other:?}"),
    };
    assert_eq!(
        failure.kind,
        FailureKind::OperationFailed,
        "malformed exact lookup must fail instead of returning an empty conversation: {failure:?}"
    );
}

/// A DM lookup without Slack's authoritative counterpart user cannot support
/// a correct mention, even when the returned conversation ID is exact.
#[tokio::test]
async fn slack_get_conversation_info_rejects_dm_without_counterpart() {
    let capability_id = CapabilityId::new("slack.get_conversation_info").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![(
        "conversations.info?channel=D0FIRAT",
        200,
        r#"{"ok":true,"channel":{"id":"D0FIRAT","is_channel":false,"is_private":true,"is_im":true,"is_mpim":false}}"#,
    )]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(network, Arc::clone(&secret_store));
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id,
            scope,
            json!({"conversation": "D0FIRAT"}),
        ))
        .await
        .unwrap();

    let failure = match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => failure,
        other => panic!("expected failed outcome, got {other:?}"),
    };
    assert_eq!(
        failure.kind,
        FailureKind::OperationFailed,
        "a DM without its authoritative counterpart must fail: {failure:?}"
    );
}

/// Slack rejects `limit=1000` (the real maximum is 999). The guest must clamp
/// vendor-out-of-range limits instead of letting the read fail on an avoidable
/// invalid_limit round-trip.
#[tokio::test]
async fn slack_history_limit_is_clamped_to_slack_maximum() {
    let capability_id = CapabilityId::new("slack.get_conversation_history").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![
        ("conversations.history", 200, SLACK_HISTORY_BODY),
        ("users.info", 200, SLACK_USER_AAA_BODY),
        ("auth.test", 200, SLACK_AUTH_TEST_SELF_AAA_BODY),
    ]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(network.clone(), Arc::clone(&secret_store));
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"conversation": "D0FIRAT", "limit": 1000}),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(_) => {}
        other => panic!("expected completed outcome, got {other:?}"),
    }
    let history_request = network
        .requests()
        .into_iter()
        .find(|request| request.url.contains("conversations.history"))
        .expect("history must call conversations.history");
    assert!(
        history_request.url.contains("limit=999"),
        "out-of-range limits must clamp to Slack's real maximum of 999: {}",
        history_request.url
    );
}

/// The canonical contract allows 1000 rows per page, while Slack's
/// conversations.list endpoint caps the value at 999. Keep the vendor-specific
/// bound inside the adapter rather than narrowing the provider-neutral schema.
#[tokio::test]
async fn slack_list_conversations_limit_is_clamped_to_slack_maximum() {
    let capability_id = CapabilityId::new("slack.list_conversations").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![(
        "conversations.list",
        200,
        r#"{"ok":true,"channels":[{"id":"C0GENERAL","name":"general","is_channel":true,"is_member":true}]}"#,
    )]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(network.clone(), Arc::clone(&secret_store));
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id,
            scope,
            json!({"limit": 1000}),
        ))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(_) => {}
        other => panic!("expected completed outcome, got {other:?}"),
    }
    let list_request = network
        .requests()
        .into_iter()
        .find(|request| request.url.contains("conversations.list"))
        .expect("list_conversations must call conversations.list");
    assert!(
        list_request.url.contains("limit=999"),
        "canonical limit=1000 must clamp to Slack's maximum: {}",
        list_request.url
    );
}

/// `search.messages` matches must carry the same author enrichment as history
/// (`author.display_name`), surface a `thread` anchor so threaded hits can be
/// followed up with slack.get_thread_replies, and decode an input `cursor`
/// into Slack's page-number paging.
#[tokio::test]
async fn slack_search_matches_carry_display_names_thread_ts_and_page() {
    let capability_id = CapabilityId::new("slack.search_messages").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![
        (
            "search.messages",
            200,
            r#"{"ok":true,"query":"deploy","messages":{"total":2,"matches":[
                {"iid":"1","channel":{"id":"C0GENERAL","name":"general"},"type":"message","user":"U0AAA","username":"firat","ts":"1751970001.000100","text":"deploy went fine per <@U0BBB>","permalink":"https://x.slack.com/archives/C0GENERAL/p1751970001000100","thread_ts":"1751960000.000100","edited":{"user":"U0AAA","ts":"1751970010.000100"}},
                {"iid":"2","channel":{"id":"C0GENERAL","name":"general"},"type":"message","user":"U0BBB","username":"ada","ts":"1751970002.000100","text":"deploying again cc <@U0QQQQQQQ|contractor.jane>","permalink":"https://x.slack.com/archives/C0GENERAL/p1751970002000100"}
            ]}}"#,
        ),
        ("users.info?user=U0AAA", 200, SLACK_USER_AAA_BODY),
        ("users.info?user=U0BBB", 200, SLACK_USER_BBB_BODY),
    ]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(network.clone(), Arc::clone(&secret_store));
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"query": "deploy", "sort": "timestamp", "cursor": "3"}),
        ))
        .await
        .unwrap();

    let output = match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => completed.output,
        other => panic!("expected completed outcome, got {other:?}"),
    };
    let search_request = network
        .requests()
        .into_iter()
        .find(|request| request.url.contains("search.messages"))
        .expect("search must call search.messages");
    assert!(
        search_request.url.contains("page=3"),
        "input cursor must decode to Slack's page paging: {}",
        search_request.url
    );
    assert!(
        search_request.url.contains("sort=timestamp")
            && search_request.url.contains("sort_dir=desc"),
        "canonical timestamp sort must map to Slack newest-first ordering: {}",
        search_request.url
    );

    let matches = output["matches"].as_array().expect("matches array");
    assert_eq!(
        matches[0]["author"]["display_name"],
        json!("Firat"),
        "search matches must resolve author display names like history does: {output}"
    );
    assert_eq!(
        matches[1]["author"]["display_name"],
        json!("Ada Lovelace"),
        "empty display_name must fall back to real_name: {output}"
    );
    assert_eq!(
        matches[0]["thread"]["thread"],
        json!("1751960000.000100"),
        "threaded search hits must surface a thread anchor for follow-up: {output}"
    );
    assert!(
        matches[1].get("thread").is_none(),
        "non-threaded matches must not fabricate a thread anchor: {output}"
    );
    assert_eq!(
        matches[0]["edited"],
        json!(true),
        "search results must preserve Slack's edited marker: {output}"
    );
    let first_text = matches[0]["text"].as_str().expect("text");
    assert!(
        first_text.contains("@Ada Lovelace") && !first_text.contains("<@U"),
        "search match text must resolve in-text mentions like history does: {output}"
    );
    // Digest/names-not-ids (qa_9c) rides the SAME resolver as quoted-message
    // hygiene (qa_10i): a labeled mention whose id can't be resolved via
    // users.info must render Slack's inline label rather than leak the raw id.
    let second_text = matches[1]["text"].as_str().expect("text");
    assert!(
        second_text.contains("@contractor.jane") && !second_text.contains("<@U0QQQQQQQ"),
        "search/digest text must fall back to the inline label for unresolvable mentions: {output}"
    );
}

/// Inbound entity hygiene (qa_10i): message text arrives with raw Slack
/// control tokens — `<@U…>` mentions, `<#C…|name>` channel refs, HTML
/// entities — and the model leaks the raw ids into user-facing replies.
/// The tool must resolve in-text mentions to `@Display Name` via the same
/// users.info cache/budget as author enrichment; a labeled mention whose id
/// can't be resolved falls back to Slack's own inline `|label` (never leaking
/// the raw id), while a bare unresolved mention stays as-is (never fabricated).
/// Channel refs render as `#name`, and &amp;/&lt;/&gt; decode.
#[tokio::test]
async fn slack_history_text_resolves_in_text_entities_to_display_names() {
    let capability_id = CapabilityId::new("slack.get_conversation_history").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![
        (
            "conversations.history",
            200,
            r#"{"ok":true,"has_more":false,"messages":[
                {"type":"message","user":"U0AAA","text":"ENTITYMSG please sync with <@U0BBB>","ts":"1751970001.000100"},
                {"type":"message","user":"U0AAA","text":"see <@U0AAA|firat> &amp; <#C0GENERAL|general> re &lt;q3&gt; <@U0ZZZZZZZ> cc <@U0ZZZZZZZ|external.person> <https://x.example/a|link>","ts":"1751970002.000100"}
            ]}"#,
        ),
        ("users.info?user=U0AAA", 200, SLACK_USER_AAA_BODY),
        ("users.info?user=U0BBB", 200, SLACK_USER_BBB_BODY),
    ]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(network.clone(), Arc::clone(&secret_store));
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"conversation": "D0FIRAT"}),
        ))
        .await
        .unwrap();

    let output = match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => completed.output,
        other => panic!("expected completed outcome, got {other:?}"),
    };
    let messages = output["messages"].as_array().expect("messages array");
    let first_text = messages[0]["text"].as_str().expect("text");
    assert!(
        first_text.contains("@Ada Lovelace"),
        "in-text mentions must resolve to display names: {output}"
    );
    assert!(
        !first_text.contains("<@U"),
        "resolved mention tokens must not leak raw user ids: {output}"
    );

    let second_text = messages[1]["text"].as_str().expect("text");
    assert!(
        second_text.contains("@Firat"),
        "labeled mention tokens (<@U…|label>) must resolve to display names: {output}"
    );
    assert!(
        second_text.contains("#general"),
        "channel refs (<#C…|name>) must render as #name: {output}"
    );
    assert!(
        second_text.contains("& ") && second_text.contains("<q3>"),
        "HTML entities (&amp; &lt; &gt;) must decode: {output}"
    );
    assert!(
        second_text.contains("<@U0ZZZZZZZ>"),
        "unresolved BARE mention tokens must stay as-is, never fabricated: {output}"
    );
    // A LABELED mention whose id can't be resolved via users.info must fall
    // back to Slack's own inline label (`@external.person`) instead of leaking
    // the raw `<@U0ZZZZZZZ|external.person>` token — the same inline-label
    // fallback the `<#C…|name>` channel-ref arm uses. This is the qa_10i leak
    // vector: a labeled mention that resolution missed still carries the raw id.
    assert!(
        second_text.contains("@external.person"),
        "labeled mention with an unresolvable id must render Slack's inline label: {output}"
    );
    assert!(
        !second_text.contains("<@U0ZZZZZZZ|"),
        "labeled unresolvable mention must not leak the raw <@U…|label> token: {output}"
    );
    assert!(
        second_text.contains("<https://x.example/a|link>"),
        "link tokens must pass through untouched: {output}"
    );

    // In-text ids ride the SAME lookup cache and budget as author
    // enrichment: U0AAA (author + labeled mention) dedupes to one lookup,
    // U0BBB and the unresolvable U0ZZZZZZZ take one attempt each.
    let lookups = network
        .requests()
        .iter()
        .filter(|request| request.url.contains("users.info"))
        .count();
    assert_eq!(
        lookups, 3,
        "in-text mention ids must share the users.info cache/budget"
    );
}

/// Wrong-identity regression (qa_10f): `chat.postMessage` with a CLASSIC
/// Slack app user token defaults to as_user=false, which attributes the
/// post to the APP (bot_id/bot_profile) instead of the connected user —
/// the probe found the mention message authored `bot: True`. The tool's
/// contract is "posts as you", so the payload must pin `as_user: true`.
#[tokio::test]
async fn slack_send_message_posts_as_the_connected_user() {
    let capability_id = CapabilityId::new("slack.send_message").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![(
        "chat.postMessage",
        200,
        r#"{"ok":true,"channel":"D0FIRAT","ts":"1751970009.000100"}"#,
    )]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(
        network.clone(),
        Arc::clone(&secret_store),
        slack_user_write_scopes()
    );
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"conversation": "D0FIRAT", "text": "hey <@U0BBB> MENTION_X"}),
        ))
        .await
        .unwrap();

    let output = match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => completed.output,
        other => panic!("expected completed outcome, got {other:?}"),
    };
    assert_eq!(
        output["message_ref"]["message_id"],
        json!("1751970009.000100")
    );

    let post_request = network
        .requests()
        .into_iter()
        .find(|request| request.url.contains("chat.postMessage"))
        .expect("send_message must call chat.postMessage");
    let body: serde_json::Value = serde_json::from_slice(&post_request.body).unwrap();
    assert_eq!(
        body["as_user"],
        json!(true),
        "user-token posts must pin as_user=true so classic apps attribute the \
         message to the connected user, not the app: {body}"
    );
    assert_eq!(body["text"], json!("hey <@U0BBB> MENTION_X"));
}

/// Granular (new) Slack apps reject the legacy as_user flag with
/// `as_user_not_supported` — their user-token posts are always authored by
/// the user — so the send must retry exactly once without the flag instead
/// of failing.
#[tokio::test]
async fn slack_send_message_retries_without_as_user_for_granular_apps() {
    let capability_id = CapabilityId::new("slack.send_message").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = SequencedSlackEgress::new(vec![
        (200, r#"{"ok":false,"error":"as_user_not_supported"}"#),
        (
            200,
            r#"{"ok":true,"channel":"D0FIRAT","ts":"1751970010.000100"}"#,
        ),
    ]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(
        network.clone(),
        Arc::clone(&secret_store),
        slack_user_write_scopes()
    );
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"conversation": "D0FIRAT", "text": "hello again"}),
        ))
        .await
        .unwrap();

    let output = match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => completed.output,
        other => panic!("expected completed outcome despite as_user rejection, got {other:?}"),
    };
    assert_eq!(
        output["message_ref"]["message_id"],
        json!("1751970010.000100")
    );

    let requests = network.requests();
    assert_eq!(
        requests.len(),
        2,
        "exactly one retry without as_user is allowed"
    );
    let first: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    let second: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(first["as_user"], json!(true));
    assert!(
        second.get("as_user").is_none(),
        "the granular-app retry must drop the legacy as_user flag: {second}"
    );
    assert_eq!(second["text"], json!("hello again"));
}

/// W3/W4 (pre-merge amendment wave): `thread` (a thread/topic container) and
/// `reply_to` (a quoted message) are distinct in the standard, but Slack has
/// only one thread-anchor mechanism (`thread_ts`) — both map onto it, and
/// both are echoed back on the output when supplied so a silent drop is
/// checkable. Two scenarios in one test (both drive the same mapping
/// contract): (1) `reply_to` alone becomes the effective `thread_ts`; (2)
/// `thread` and `reply_to` together, naming different messages — `thread`
/// wins for the vendor call, and both are still echoed.
#[tokio::test]
async fn slack_send_message_reply_to_maps_onto_thread_ts_and_echoes_both() {
    let capability_id = CapabilityId::new("slack.send_message").unwrap();

    // Scenario 1: reply_to alone.
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![(
        "chat.postMessage",
        200,
        r#"{"ok":true,"channel":"D0FIRAT","ts":"1751970020.000100"}"#,
    )]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(
        network.clone(),
        Arc::clone(&secret_store),
        slack_user_write_scopes()
    );
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({
                "conversation": "D0FIRAT",
                "text": "quoting you",
                "reply_to": {"conversation": "D0FIRAT", "message_id": "1751970001.000100"},
            }),
        ))
        .await
        .unwrap();
    let output = match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => completed.output,
        other => panic!("expected completed outcome, got {other:?}"),
    };
    let post_request = network
        .requests()
        .into_iter()
        .find(|request| request.url.contains("chat.postMessage"))
        .expect("send_message must call chat.postMessage");
    let body: serde_json::Value = serde_json::from_slice(&post_request.body).unwrap();
    assert_eq!(
        body["thread_ts"],
        json!("1751970001.000100"),
        "reply_to alone must become the effective thread_ts: {body}"
    );
    assert_eq!(
        output["reply_to"],
        json!({"conversation": "D0FIRAT", "message_id": "1751970001.000100"}),
        "reply_to must echo back on the output: {output}"
    );
    assert!(
        output.get("thread").is_none(),
        "thread must not be fabricated when only reply_to was supplied: {output}"
    );

    // Scenario 2: thread AND reply_to, naming different messages — thread
    // wins for the vendor call; both are still echoed on the output.
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![(
        "chat.postMessage",
        200,
        r#"{"ok":true,"channel":"D0FIRAT","ts":"1751970021.000100"}"#,
    )]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(
        network.clone(),
        Arc::clone(&secret_store),
        slack_user_write_scopes()
    );
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id,
            scope,
            json!({
                "conversation": "D0FIRAT",
                "text": "in the thread",
                "thread": "1751960000.000100",
                "reply_to": {"conversation": "D0FIRAT", "message_id": "1751970001.000100"},
            }),
        ))
        .await
        .unwrap();
    let output = match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => completed.output,
        other => panic!("expected completed outcome, got {other:?}"),
    };
    let post_request = network
        .requests()
        .into_iter()
        .find(|request| request.url.contains("chat.postMessage"))
        .expect("send_message must call chat.postMessage");
    let body: serde_json::Value = serde_json::from_slice(&post_request.body).unwrap();
    assert_eq!(
        body["thread_ts"],
        json!("1751960000.000100"),
        "thread must win over reply_to for the vendor call when they disagree: {body}"
    );
    assert_eq!(
        output["thread"],
        json!("1751960000.000100"),
        "thread must echo back on the output: {output}"
    );
    assert_eq!(
        output["reply_to"],
        json!({"conversation": "D0FIRAT", "message_id": "1751970001.000100"}),
        "reply_to must still echo back on the output even though thread won: {output}"
    );
}

/// W14 (pre-merge amendment wave, binding ruling): a cursor that fails to
/// decode as a Slack page number must surface a model-visible
/// `messaging.unsupported_content` error and must NEVER silently restart the
/// search at page 1 — proven here by asserting zero requests reached the
/// vendor at all.
#[tokio::test]
async fn slack_search_messages_rejects_garbled_cursor_without_calling_the_vendor() {
    let capability_id = CapabilityId::new("slack.search_messages").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![(
        "search.messages",
        200,
        r#"{"ok":true,"messages":{"total":0,"matches":[]}}"#,
    )]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(network.clone(), Arc::clone(&secret_store));
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id,
            scope,
            json!({"query": "deploy", "cursor": "not-a-page-number"}),
        ))
        .await
        .unwrap();

    let failure = match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => failure,
        other => panic!("expected failed outcome, got {other:?}"),
    };
    assert_eq!(
        failure.kind,
        FailureKind::InputEncode,
        "an undecodable cursor is model-fixable: {failure:?}"
    );
    let message = failure.message.as_deref().unwrap_or_default();
    assert!(
        message.contains("messaging.unsupported_content"),
        "an undecodable cursor must surface unsupported_content, got: {message:?}"
    );
    assert!(
        network.requests().is_empty(),
        "a garbled cursor must be rejected before any vendor HTTP call: {:?}",
        network.requests()
    );
}

/// W16 (pre-merge amendment wave, verified defect): Slack's `no_text` means
/// the message body's CONTENT was rejected (empty after formatting), never a
/// length limit — it must map to `messaging.unsupported_content`, not
/// `messaging.message_too_long` (the inverted mapping this fixes).
#[tokio::test]
async fn slack_no_text_surfaces_unsupported_content_not_message_too_long() {
    let capability_id = CapabilityId::new("slack.send_message").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![(
        "chat.postMessage",
        200,
        r#"{"ok":false,"error":"no_text"}"#,
    )]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(
        network.clone(),
        Arc::clone(&secret_store),
        slack_user_write_scopes()
    );
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id,
            scope,
            json!({"conversation": "D0FIRAT", "text": "."}),
        ))
        .await
        .unwrap();

    let failure = match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => failure,
        other => panic!("expected failed outcome, got {other:?}"),
    };
    assert_eq!(
        failure.kind,
        FailureKind::InputEncode,
        "no_text is model-correctable input, not a run-ending host error: {failure:?}"
    );
    let message = failure.message.as_deref().unwrap_or_default();
    assert!(
        message.contains("messaging.unsupported_content"),
        "no_text must map to unsupported_content, got: {message:?}"
    );
    assert!(
        !message.contains("messaging.message_too_long"),
        "no_text must no longer map to message_too_long (the inverted mapping this fixes), got: {message:?}"
    );
    assert!(
        !message.contains("no_text"),
        "the raw vendor code must never leak into the model-visible message: {message:?}"
    );
}

/// Enrichment is best-effort: a failing `users.info` (missing scope, outage)
/// must never break the read itself — raw ids still return, names are simply
/// absent.
#[tokio::test]
async fn slack_history_read_survives_users_info_failure_without_names() {
    let capability_id = CapabilityId::new("slack.get_conversation_history").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![
        ("conversations.history", 200, SLACK_HISTORY_BODY),
        ("users.info", 200, r#"{"ok":false,"error":"missing_scope"}"#),
    ]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(network.clone(), Arc::clone(&secret_store));
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"conversation": "D0FIRAT"}),
        ))
        .await
        .unwrap();

    let output = match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => completed.output,
        other => panic!("expected completed outcome despite users.info failure, got {other:?}"),
    };
    let messages = output["messages"].as_array().expect("messages array");
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["author"]["user_ref"], json!("U0AAA"));
    assert!(
        messages[0]["author"].get("display_name").is_none(),
        "no display name should be fabricated when users.info fails: {output}"
    );
    // auth.test is unscripted here (fixture 404s it): identity marking must
    // degrade to `false` — `is_self` is a required, always-concrete bool
    // (never absent, never a fabricated `true`) — never a failed read.
    assert_eq!(
        messages[0]["is_self"],
        json!(false),
        "is_self must default to false when the connected identity is unknown: {output}"
    );
}

/// Identity-attribution regression, part 2: the model must be able to ask
/// "who am I on Slack?" directly. `slack.whoami` resolves the CONNECTED
/// account via `auth.test` + a best-effort `users.info` for the display name.
#[tokio::test]
async fn slack_whoami_resolves_connected_identity() {
    let capability_id = CapabilityId::new("slack.whoami").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![
        ("auth.test", 200, SLACK_AUTH_TEST_SELF_AAA_BODY),
        ("users.info?user=U0AAA", 200, SLACK_USER_AAA_BODY),
    ]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(network.clone(), Arc::clone(&secret_store));
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({}),
        ))
        .await
        .unwrap();

    let output = match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => completed.output,
        other => panic!("expected completed outcome, got {other:?}"),
    };
    assert_eq!(
        output["user_ref"],
        json!("U0AAA"),
        "whoami must return the connected account's user id: {output}"
    );
    assert_eq!(
        output["display_name"],
        json!("Firat"),
        "whoami must resolve the connected account's display name: {output}"
    );
}

/// Structured guest errors: a Slack `ok:false` code must reach the model as
/// actionable failure detail, not be erased to a generic "the tool operation
/// failed". Slack's `channel_not_found` maps onto the standardized messaging
/// framework's closed error taxonomy (`messaging.unknown_conversation`) —
/// model-fixable (list conversations, pick another id) — so it must classify
/// as InvalidInput AND carry the STANDARD code (never the raw Slack code) in
/// the model-visible failure message.
#[tokio::test]
async fn slack_channel_not_found_surfaces_code_in_model_visible_failure() {
    let capability_id = CapabilityId::new("slack.get_conversation_history").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![(
        "conversations.history",
        200,
        r#"{"ok":false,"error":"channel_not_found"}"#,
    )]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(network.clone(), Arc::clone(&secret_store));
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"conversation": "C0MISSING"}),
        ))
        .await
        .unwrap();

    let failure = match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => failure,
        other => panic!("expected failed outcome, got {other:?}"),
    };
    assert_eq!(
        failure.kind,
        FailureKind::InputEncode,
        "channel_not_found is a model-fixable input error: {failure:?}"
    );
    let message = failure.message.as_deref().unwrap_or_default();
    assert!(
        message.contains("messaging.unknown_conversation"),
        "model-visible failure detail must carry the STANDARD taxonomy code, got: {message:?}"
    );
    assert!(
        !message.contains("channel_not_found"),
        "the raw vendor code must never leak into the model-visible message: {message:?}"
    );
}

const SLACK_USER_STATUS_BODY: &str = r#"{"ok":true,"user":{"id":"U0CCC","name":"george","is_bot":false,"tz":"America/New_York","tz_label":"Eastern Daylight Time","profile":{"display_name":"George","real_name":"George Harrison","title":"Guitarist","status_text":"On vacation until July 20","status_emoji":":palm_tree:","status_expiration":1753027199}}}"#;

/// Presence honesty: `get_user_info` must surface the profile fields an
/// "is George around?" question actually needs — Slack status text/emoji,
/// timezone, title — instead of hiding them and letting the model guess.
/// `tz_label` is a deliberate drop (product decision: redundant with the
/// IANA `timezone`, no canonical field). `status_expiration` is NOT
/// redundant — it is the only machine-readable way to know when a status
/// auto-clears (`status_text` does not encode it) — so it rides the
/// canonical schema's optional `vendor: object` passthrough instead of
/// being dropped.
#[tokio::test]
async fn slack_get_user_info_surfaces_status_and_timezone() {
    let capability_id = CapabilityId::new("slack.get_user_info").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network =
        UrlKeyedSlackEgress::new(vec![("users.info?user=U0CCC", 200, SLACK_USER_STATUS_BODY)]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(network.clone(), Arc::clone(&secret_store));
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"user_ref": "U0CCC"}),
        ))
        .await
        .unwrap();

    let output = match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => completed.output,
        other => panic!("expected completed outcome, got {other:?}"),
    };
    assert_eq!(
        output["status_text"],
        json!("On vacation until July 20"),
        "get_user_info must surface the Slack status text: {output}"
    );
    assert_eq!(output["status_emoji"], json!(":palm_tree:"));
    assert_eq!(
        output["timezone"],
        json!("America/New_York"),
        "get_user_info must surface the user's timezone: {output}"
    );
    assert_eq!(output["title"], json!("Guitarist"));
    assert!(
        output.get("tz_label").is_none() && output.get("status_expiration").is_none(),
        "fields with no canonical home must not leak into the top level: {output}"
    );
    assert_eq!(
        output["vendor"]["status_expiration"],
        json!(1_753_027_199),
        "status_expiration must pass through under the schema's vendor extras: {output}"
    );
}

/// Review follow-up (PR #5898): name resolution is capped so a busy channel
/// cannot turn one history read into dozens of sequential `users.info`
/// round-trips (the WASM tool is synchronous). First-seen authors win the
/// budget; authors past the cap keep raw ids only — the same degraded shape
/// as a failed lookup, which the model already tolerates.
#[tokio::test]
async fn slack_history_name_resolution_is_capped_per_call() {
    let capability_id = CapabilityId::new("slack.get_conversation_history").unwrap();
    let scope = sample_scope(InvocationId::new());
    let mut messages = Vec::new();
    for index in 0..30 {
        messages.push(format!(
            r#"{{"type":"message","user":"U{index:04}","text":"m{index}","ts":"1751970{index:03}.000100"}}"#
        ));
    }
    let history_body = Box::leak(
        format!(
            r#"{{"ok":true,"has_more":false,"messages":[{}]}}"#,
            messages.join(",")
        )
        .into_boxed_str(),
    );
    let network = UrlKeyedSlackEgress::new(vec![
        ("conversations.history", 200, history_body),
        ("users.info", 200, SLACK_USER_AAA_BODY),
    ]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(network.clone(), Arc::clone(&secret_store));
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id.clone(),
            scope,
            json!({"conversation": "C0BUSY"}),
        ))
        .await
        .unwrap();

    let output = match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => completed.output,
        other => panic!("expected completed outcome, got {other:?}"),
    };
    let lookups = network
        .requests()
        .iter()
        .filter(|request| request.url.contains("users.info"))
        .count();
    assert_eq!(
        lookups, 25,
        "users.info lookups must be capped at 25 per call, first-seen order"
    );
    let messages = output["messages"].as_array().expect("messages array");
    assert!(
        messages[24]["author"].get("display_name").is_some(),
        "authors inside the lookup budget must be named: {output}"
    );
    assert!(
        messages[25]["author"].get("display_name").is_none(),
        "authors past the lookup budget keep raw ids only: {output}"
    );
}

// ─── Standardized messaging framework conformance (task 9, brief step 6) ───
//
// Every Slack standard op, driven through the REAL bundled
// `slack_user_tool.wasm` via `invoke_capability` (the same seam every other
// Slack test in this file exercises), must produce output that satisfies its
// canonical output schema — the exact contract the host's post-dispatch
// output enforcement (task 5, `ironclaw_host_runtime::standard_op_output`)
// validates against in production. `assert_canonical_output` panics loudly
// on any schema violation, so a bare "did not panic" is the pass condition
// for each op.

/// Drive one Slack standard op through the real bundled WASM module and
/// return its output on success — the shared harness the conformance sweep
/// below uses for every bound op (all 16 core standard messaging ops today)
/// instead of repeating the ~15-line registry/network/secret-store setup
/// per op.
async fn dispatch_slack_standard_op_for_conformance(
    capability_name: &str,
    scopes: Vec<String>,
    scripted: Vec<(&'static str, u16, &'static str)>,
    input: serde_json::Value,
) -> serde_json::Value {
    let capability_id = CapabilityId::new(capability_name).unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(scripted);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(
        network.clone(),
        Arc::clone(&secret_store),
        scopes.clone()
    );
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(capability_id, scope, input))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => completed.output,
        other => panic!("{capability_name}: expected completed outcome, got {other:?}"),
    }
}

/// Slack can return partially populated rows for message subtypes. Rows that
/// lack canonical identity must be omitted instead of fabricating empty
/// message, conversation, or author references that invalidate the entire
/// capability result.
#[tokio::test]
async fn slack_message_reads_drop_rows_without_canonical_identity() {
    let history_output = dispatch_slack_standard_op_for_conformance(
        "slack.get_conversation_history",
        slack_user_scopes(),
        vec![(
            "conversations.history",
            200,
            r#"{"ok":true,"messages":[
                {"type":"message","user":"U0AAA","text":"valid","ts":"1751970001.000100"},
                {"type":"message","text":"missing author","ts":"1751970002.000100"},
                {"type":"message","user":"U0BBB","text":"missing timestamp"}
            ]}"#,
        )],
        json!({"conversation": "C0GENERAL"}),
    )
    .await;
    assert_eq!(
        history_output["messages"].as_array().map(Vec::len),
        Some(1),
        "history must omit rows that cannot satisfy canonical identity: {history_output}"
    );

    let search_output = dispatch_slack_standard_op_for_conformance(
        "slack.search_messages",
        slack_user_scopes(),
        vec![(
            "search.messages",
            200,
            r#"{"ok":true,"messages":{"matches":[
                {"channel":{"id":"C0GENERAL"},"user":"U0AAA","ts":"1751970001.000100","text":"valid"},
                {"channel":{},"user":"U0BBB","ts":"1751970002.000100","text":"missing conversation"},
                {"channel":{"id":"C0GENERAL"},"ts":"1751970003.000100","text":"missing author"}
            ]}}"#,
        )],
        json!({"query": "valid"}),
    )
    .await;
    assert_eq!(
        search_output["matches"].as_array().map(Vec::len),
        Some(1),
        "search must omit rows that cannot satisfy canonical identity: {search_output}"
    );
}

#[tokio::test]
async fn slack_standard_ops_satisfy_canonical_contracts() {
    use ironclaw_host_api::messaging::StandardMessagingOp;
    use ironclaw_host_api::test_support::messaging_conformance::assert_canonical_output;

    let read_scopes = slack_user_scopes();
    let write_scopes = slack_user_write_scopes();

    let send_message_output = dispatch_slack_standard_op_for_conformance(
        "slack.send_message",
        write_scopes.clone(),
        vec![(
            "chat.postMessage",
            200,
            r#"{"ok":true,"channel":"C0GENERAL","ts":"1751970099.000100"}"#,
        )],
        json!({"conversation": "C0GENERAL", "text": "hello"}),
    )
    .await;
    assert_canonical_output(StandardMessagingOp::SendMessage, &send_message_output);

    let search_messages_output = dispatch_slack_standard_op_for_conformance(
        "slack.search_messages",
        read_scopes.clone(),
        vec![(
            "search.messages",
            200,
            r#"{"ok":true,"messages":{"total":1,"matches":[
                {"channel":{"id":"C0GENERAL"},"user":"U0AAA","ts":"1751970001.000100","text":"hi"}
            ]}}"#,
        )],
        json!({"query": "hello"}),
    )
    .await;
    assert_canonical_output(StandardMessagingOp::SearchMessages, &search_messages_output);

    let list_conversations_output = dispatch_slack_standard_op_for_conformance(
        "slack.list_conversations",
        read_scopes.clone(),
        vec![(
            "conversations.list",
            200,
            r#"{"ok":true,"channels":[
                {"id":"C0GENERAL","name":"general","is_channel":true,"is_private":false,"is_im":false,"is_mpim":false,"is_member":true}
            ]}"#,
        )],
        json!({}),
    )
    .await;
    assert_canonical_output(
        StandardMessagingOp::ListConversations,
        &list_conversations_output,
    );

    let get_conversation_info_output = dispatch_slack_standard_op_for_conformance(
        "slack.get_conversation_info",
        read_scopes.clone(),
        vec![(
            "conversations.info?channel=C0GENERAL",
            200,
            r#"{"ok":true,"channel":{"id":"C0GENERAL","name":"general","is_channel":true,"is_private":false,"is_im":false,"is_mpim":false,"is_member":true}}"#,
        )],
        json!({"conversation": "C0GENERAL"}),
    )
    .await;
    assert_canonical_output(
        StandardMessagingOp::GetConversationInfo,
        &get_conversation_info_output,
    );

    let get_conversation_history_output = dispatch_slack_standard_op_for_conformance(
        "slack.get_conversation_history",
        read_scopes.clone(),
        vec![(
            "conversations.history",
            200,
            r#"{"ok":true,"messages":[{"type":"message","user":"U0AAA","text":"hi","ts":"1751970001.000100"}]}"#,
        )],
        json!({"conversation": "C0GENERAL"}),
    )
    .await;
    assert_canonical_output(
        StandardMessagingOp::GetConversationHistory,
        &get_conversation_history_output,
    );

    let get_thread_replies_output = dispatch_slack_standard_op_for_conformance(
        "slack.get_thread_replies",
        read_scopes.clone(),
        vec![(
            "conversations.replies",
            200,
            r#"{"ok":true,"messages":[{"type":"message","user":"U0AAA","text":"hi","ts":"1751970001.000100","thread_ts":"1751970001.000100"}]}"#,
        )],
        json!({"conversation": "C0GENERAL", "thread": "1751970001.000100"}),
    )
    .await;
    assert_canonical_output(
        StandardMessagingOp::GetThreadReplies,
        &get_thread_replies_output,
    );

    let get_user_info_output = dispatch_slack_standard_op_for_conformance(
        "slack.get_user_info",
        read_scopes.clone(),
        vec![("users.info?user=U0AAA", 200, SLACK_USER_AAA_BODY)],
        json!({"user_ref": "U0AAA"}),
    )
    .await;
    assert_canonical_output(StandardMessagingOp::GetUserInfo, &get_user_info_output);
    assert!(
        get_user_info_output.get("vendor").is_none(),
        "get_user_info: no vendor key when Slack reports no status_expiration: {get_user_info_output}"
    );

    // Slack's status auto-clear timestamp has no canonical field but is not
    // redundant (status_text does not encode it): it must pass through the
    // schema's `vendor` passthrough rather than being dropped.
    let get_user_info_with_status_expiration_output = dispatch_slack_standard_op_for_conformance(
        "slack.get_user_info",
        read_scopes.clone(),
        vec![("users.info?user=U0CCC", 200, SLACK_USER_STATUS_BODY)],
        json!({"user_ref": "U0CCC"}),
    )
    .await;
    assert_canonical_output(
        StandardMessagingOp::GetUserInfo,
        &get_user_info_with_status_expiration_output,
    );
    assert_eq!(
        get_user_info_with_status_expiration_output["vendor"]["status_expiration"],
        json!(1_753_027_199),
        "get_user_info: status_expiration must pass through under vendor: {get_user_info_with_status_expiration_output}"
    );

    let whoami_output = dispatch_slack_standard_op_for_conformance(
        "slack.whoami",
        read_scopes.clone(),
        vec![
            ("auth.test", 200, SLACK_AUTH_TEST_SELF_AAA_BODY),
            ("users.info?user=U0AAA", 200, SLACK_USER_AAA_BODY),
        ],
        json!({}),
    )
    .await;
    assert_canonical_output(StandardMessagingOp::Whoami, &whoami_output);

    // ── The eight ops the scope-widening wave added (edit/delete/reactions/
    //    open_dm/get_message/people). Same seam, same pass condition.

    let edit_message_output = dispatch_slack_standard_op_for_conformance(
        "slack.edit_message",
        write_scopes.clone(),
        vec![(
            "chat.update",
            200,
            r#"{"ok":true,"channel":"C0GENERAL","ts":"1751970099.000100"}"#,
        )],
        json!({
            "message_ref": {"conversation": "C0GENERAL", "message_id": "1751970099.000100"},
            "text": "edited body"
        }),
    )
    .await;
    assert_canonical_output(StandardMessagingOp::EditMessage, &edit_message_output);

    let delete_message_output = dispatch_slack_standard_op_for_conformance(
        "slack.delete_message",
        write_scopes,
        vec![(
            "chat.delete",
            200,
            r#"{"ok":true,"channel":"C0GENERAL","ts":"1751970099.000100"}"#,
        )],
        json!({
            "message_ref": {"conversation": "C0GENERAL", "message_id": "1751970099.000100"}
        }),
    )
    .await;
    assert_canonical_output(StandardMessagingOp::DeleteMessage, &delete_message_output);
    assert_eq!(
        delete_message_output["deleted"],
        json!(true),
        "a delete that happened reports deleted: true: {delete_message_output}"
    );

    let add_reaction_output = dispatch_slack_standard_op_for_conformance(
        "slack.add_reaction",
        slack_user_scopes_plus(&["reactions:write"]),
        vec![("reactions.add", 200, r#"{"ok":true}"#)],
        json!({
            "message_ref": {"conversation": "C0GENERAL", "message_id": "1751970001.000100"},
            "emoji": ":thumbsup:"
        }),
    )
    .await;
    assert_canonical_output(StandardMessagingOp::AddReaction, &add_reaction_output);
    assert_eq!(
        add_reaction_output["emoji"],
        json!("thumbsup"),
        "wrapping colons are normalized away before Slack sees the name: {add_reaction_output}"
    );

    let remove_reaction_output = dispatch_slack_standard_op_for_conformance(
        "slack.remove_reaction",
        slack_user_scopes_plus(&["reactions:read", "reactions:write"]),
        vec![("reactions.remove", 200, r#"{"ok":true}"#)],
        json!({
            "message_ref": {"conversation": "C0GENERAL", "message_id": "1751970001.000100"},
            "emoji": "thumbsup"
        }),
    )
    .await;
    assert_canonical_output(StandardMessagingOp::RemoveReaction, &remove_reaction_output);
    assert_eq!(
        remove_reaction_output["emoji"],
        json!("thumbsup"),
        "the named-emoji variant echoes what it removed: {remove_reaction_output}"
    );

    let open_dm_output = dispatch_slack_standard_op_for_conformance(
        "slack.open_dm",
        slack_user_scopes_plus(&["im:write"]),
        vec![(
            "conversations.open",
            200,
            r#"{"ok":true,"channel":{"id":"D0NEWDM"}}"#,
        )],
        json!({"user_ref": "U0BBB"}),
    )
    .await;
    assert_canonical_output(StandardMessagingOp::OpenDm, &open_dm_output);
    assert_eq!(
        open_dm_output["conversation"],
        json!("D0NEWDM"),
        "open_dm returns the provider-issued DM conversation: {open_dm_output}"
    );

    let get_message_output = dispatch_slack_standard_op_for_conformance(
        "slack.get_message",
        read_scopes.clone(),
        vec![
            (
                "conversations.history",
                200,
                r#"{"ok":true,"messages":[{"type":"message","user":"U0AAA","text":"hi","ts":"1751970001.000100"}]}"#,
            ),
            ("users.info?user=U0AAA", 200, SLACK_USER_AAA_BODY),
            ("auth.test", 200, SLACK_AUTH_TEST_SELF_AAA_BODY),
        ],
        json!({
            "message_ref": {"conversation": "C0GENERAL", "message_id": "1751970001.000100"}
        }),
    )
    .await;
    assert_canonical_output(StandardMessagingOp::GetMessage, &get_message_output);

    let resolve_user_output = dispatch_slack_standard_op_for_conformance(
        "slack.resolve_user",
        read_scopes.clone(),
        vec![(
            "users.list",
            200,
            r#"{"ok":true,"members":[{"id":"U0BBB","name":"bob","profile":{"real_name":"Bob Example","display_name":"Bob"}}],"response_metadata":{"next_cursor":""}}"#,
        )],
        json!({"query": "bob"}),
    )
    .await;
    assert_canonical_output(StandardMessagingOp::ResolveUser, &resolve_user_output);
    assert_eq!(
        resolve_user_output["matches"][0]["user_ref"],
        json!("U0BBB"),
        "a directory match carries the mentionable user id: {resolve_user_output}"
    );

    let list_members_output = dispatch_slack_standard_op_for_conformance(
        "slack.list_members",
        read_scopes,
        vec![
            (
                "conversations.members",
                200,
                r#"{"ok":true,"members":["U0AAA"],"response_metadata":{"next_cursor":""}}"#,
            ),
            ("users.info?user=U0AAA", 200, SLACK_USER_AAA_BODY),
        ],
        json!({"conversation": "C0GENERAL"}),
    )
    .await;
    assert_canonical_output(StandardMessagingOp::ListMembers, &list_members_output);
}

/// One vendor-error case (brief step 6): a scripted Slack `not_in_channel`
/// response must surface the STANDARD taxonomy code `messaging.not_a_member`
/// in the model-visible failure — never the raw Slack code, never a silent
/// pass-through.
#[tokio::test]
async fn slack_standard_op_vendor_error_surfaces_not_a_member() {
    let capability_id = CapabilityId::new("slack.get_conversation_history").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![(
        "conversations.history",
        200,
        r#"{"ok":false,"error":"not_in_channel"}"#,
    )]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(network, Arc::clone(&secret_store));
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id,
            scope,
            json!({"conversation": "C0PRIVATE"}),
        ))
        .await
        .unwrap();

    let failure = match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => failure,
        other => panic!("expected failed outcome, got {other:?}"),
    };
    assert_eq!(
        failure.kind,
        FailureKind::OperationFailed,
        "not_in_channel is a model-visible vendor denial: {failure:?}"
    );
    let message = failure.message.as_deref().unwrap_or_default();
    assert!(
        message.contains("messaging.not_a_member"),
        "not_in_channel must surface the standard taxonomy code, got: {message:?}"
    );
    assert!(
        !message.contains("not_in_channel"),
        "the raw vendor code must never leak into the model-visible message: {message:?}"
    );
}

// ─── Behavioral contracts of the eight scope-widening ops ──────────────────
//
// Everything below drives the REAL bundled `slack_user_tool.wasm` through
// `invoke_capability`, exactly like the conformance sweep — these tests pin
// the orchestration each op performs (which vendor calls, in what order,
// with which query parameters) and the model-visible failures, which the
// output-schema sweep cannot see.

/// The message every reaction/get test addresses.
fn general_message_ref() -> serde_json::Value {
    json!({"conversation": "C0GENERAL", "message_id": "1751970001.000100"})
}

/// The failure-path sibling of [`dispatch_slack_standard_op_for_conformance`]:
/// drive one Slack standard op expecting a model-visible failure, returning
/// that failure PLUS the recorded egress so callers can assert exactly which
/// vendor calls happened — including none at all, the contract for inputs the
/// guest must reject before Slack is ever asked.
async fn dispatch_slack_standard_op_expecting_failure(
    capability_name: &str,
    scopes: Vec<String>,
    scripted: Vec<(&'static str, u16, &'static str)>,
    input: serde_json::Value,
) -> (RuntimeCapabilityFailure, UrlKeyedSlackEgress) {
    let capability_id = CapabilityId::new(capability_name).unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(scripted);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(
        network.clone(),
        Arc::clone(&secret_store),
        scopes.clone()
    );
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(capability_id, scope, input))
        .await
        .unwrap();

    match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => (failure, network),
        other => panic!("{capability_name}: expected failed outcome, got {other:?}"),
    }
}

/// Slack-side scope drift surfaces as the model-visible standard denial,
/// never as an auth-required re-auth gate.
///
/// Two layers gate a scope shortfall, and this test pins the second one:
///
/// 1. An account whose STORED grant lacks a tool's manifest scopes (the
///    pre-widening account) never reaches Slack at all — production account
///    selection applies the provider-scope gate
///    (`AccountSelectionPurpose::Runtime` in
///    `ironclaw_auth::product_auth::credentials::runtime_credentials`), maps
///    the empty selection to `CredentialStageError::AuthRequired`, and the
///    re-auth gate reconnects the SAME account with the widened manifest
///    scopes (binding deliberately skips the scope gate).
/// 2. An account that PASSED staging can still be refused by Slack with
///    `missing_scope` (server-side drift: a scope revoked after connect, an
///    org policy). Re-running OAuth would request the exact scopes the
///    account already claims to hold, so the guest maps this to
///    `messaging.permission_denied` — a model-visible denial the model can
///    explain — instead of a re-auth loop (`slack_error_requires_reauth`).
#[tokio::test]
async fn slack_missing_scope_surfaces_permission_denied_not_auth_required() {
    let (failure, network) = dispatch_slack_standard_op_expecting_failure(
        "slack.add_reaction",
        // The token staged WITH its manifest scopes — staging passes and the
        // shortfall comes from Slack itself.
        slack_user_scopes_plus(&["reactions:write"]),
        vec![(
            "reactions.add",
            200,
            r#"{"ok":false,"error":"missing_scope"}"#,
        )],
        json!({"message_ref": general_message_ref(), "emoji": "thumbsup"}),
    )
    .await;
    // Reaching here at all proves the outcome was Failed — the helper panics
    // on RuntimeCapabilityOutcome::AuthRequired, the wrong-answer this test
    // exists to rule out.
    assert_eq!(
        failure.kind,
        FailureKind::OperationFailed,
        "a scope shortfall is a model-visible vendor denial: {failure:?}"
    );
    let message = failure.message.as_deref().unwrap_or_default();
    assert!(
        message.contains("messaging.permission_denied"),
        "missing_scope must surface the standard taxonomy code, got: {message:?}"
    );
    assert!(
        !message.contains("missing_scope"),
        "the raw vendor code must never leak into the model-visible message: {message:?}"
    );
    assert_eq!(
        network.requests().len(),
        1,
        "exactly one vendor call answers a scope shortfall"
    );
}

/// The idempotent end-state arms through dispatch: Slack's "already reacted" /
/// "that reaction is not there" answers mean the requested end state holds,
/// so both complete successfully instead of pushing the model into retrying
/// a call that cannot change anything.
#[tokio::test]
async fn slack_reaction_end_state_codes_complete_successfully() {
    use ironclaw_host_api::messaging::StandardMessagingOp;
    use ironclaw_host_api::test_support::messaging_conformance::assert_canonical_output;

    let already_reacted_output = dispatch_slack_standard_op_for_conformance(
        "slack.add_reaction",
        slack_user_scopes_plus(&["reactions:write"]),
        vec![(
            "reactions.add",
            200,
            r#"{"ok":false,"error":"already_reacted"}"#,
        )],
        json!({"message_ref": general_message_ref(), "emoji": "thumbsup"}),
    )
    .await;
    assert_canonical_output(StandardMessagingOp::AddReaction, &already_reacted_output);

    let no_reaction_output = dispatch_slack_standard_op_for_conformance(
        "slack.remove_reaction",
        slack_user_scopes_plus(&["reactions:read", "reactions:write"]),
        vec![(
            "reactions.remove",
            200,
            r#"{"ok":false,"error":"no_reaction"}"#,
        )],
        json!({"message_ref": general_message_ref(), "emoji": "thumbsup"}),
    )
    .await;
    assert_canonical_output(StandardMessagingOp::RemoveReaction, &no_reaction_output);
}

/// The omit-emoji `remove_reaction` orchestration, end to end: resolve the
/// connected identity (`auth.test`), read the message's reactions with the
/// COMPLETE user lists (`reactions.get?...&full=true` — without `full=true`
/// Slack truncates each reaction's `users` array on popular messages and a
/// reaction that IS ours would be silently skipped as a false success), then
/// remove exactly the connected account's reactions and nobody else's.
#[tokio::test]
async fn slack_remove_reaction_without_emoji_removes_only_the_connected_accounts_reactions() {
    use ironclaw_host_api::messaging::StandardMessagingOp;
    use ironclaw_host_api::test_support::messaging_conformance::assert_canonical_output;

    let capability_id = CapabilityId::new("slack.remove_reaction").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![
        ("auth.test", 200, SLACK_AUTH_TEST_SELF_AAA_BODY),
        (
            "reactions.get",
            200,
            r#"{"ok":true,"message":{"reactions":[
                {"name":"thumbsup","users":["U0AAA","U0BBB"]},
                {"name":"tada","users":["U0BBB"]},
                {"name":"eyes","users":["U0AAA"]}
            ]}}"#,
        ),
        ("reactions.remove", 200, r#"{"ok":true}"#),
    ]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(
        network.clone(),
        Arc::clone(&secret_store),
        slack_user_scopes_plus(&["reactions:read", "reactions:write"])
    );
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id,
            scope,
            json!({"message_ref": general_message_ref()}),
        ))
        .await
        .unwrap();

    let output = match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => completed.output,
        other => panic!("expected completed outcome, got {other:?}"),
    };
    assert_canonical_output(StandardMessagingOp::RemoveReaction, &output);
    assert!(
        output.get("emoji").is_none(),
        "the omit-emoji variant names no single emoji on the way out: {output}"
    );

    let requests = network.requests();
    let urls: Vec<&str> = requests.iter().map(|r| r.url.as_str()).collect();
    let auth_index = urls
        .iter()
        .position(|u| u.contains("auth.test"))
        .expect("identity is resolved first");
    let get_index = urls
        .iter()
        .position(|u| u.contains("reactions.get"))
        .expect("the message's reactions are read");
    assert!(
        auth_index < get_index,
        "identity before the reactions read: {urls:?}"
    );
    assert!(
        urls[get_index].contains("full=true"),
        "reactions.get must request the complete, untruncated user lists: {}",
        urls[get_index]
    );

    let removed_names: Vec<String> = requests
        .iter()
        .filter(|r| r.url.contains("reactions.remove"))
        .map(|r| {
            serde_json::from_slice::<serde_json::Value>(&r.body).expect("remove body is JSON")
                ["name"]
                .as_str()
                .expect("remove body names its emoji")
                .to_string()
        })
        .collect();
    assert_eq!(
        removed_names,
        vec!["thumbsup".to_string(), "eyes".to_string()],
        "exactly the connected account's reactions are removed, never anyone else's"
    );
}

/// The identity lookup in the omit-emoji variant is fatal, not best-effort:
/// without knowing who "we" are, the ownership filter could remove someone
/// else's reaction — the one outcome this operation must never produce. A
/// failed `auth.test` therefore aborts before reactions.get or any removal.
#[tokio::test]
async fn slack_remove_reaction_without_emoji_fails_closed_when_identity_lookup_fails() {
    let (failure, network) = dispatch_slack_standard_op_expecting_failure(
        "slack.remove_reaction",
        slack_user_scopes_plus(&["reactions:read", "reactions:write"]),
        vec![(
            "auth.test",
            200,
            r#"{"ok":false,"error":"unexpected_glitch"}"#,
        )],
        json!({"message_ref": general_message_ref()}),
    )
    .await;
    assert_eq!(
        failure.kind,
        FailureKind::OperationFailed,
        "a degraded identity fails the call instead of degrading the filter: {failure:?}"
    );
    let requests = network.requests();
    assert_eq!(
        requests.len(),
        1,
        "the identity failure must abort before reactions.get or any removal: {requests:?}"
    );
    assert!(
        requests[0].url.contains("auth.test"),
        "the one call made is the identity probe: {}",
        requests[0].url
    );
}

/// `conversations.history`'s `latest` is a RANGE bound: for a ts that no
/// longer resolves it hands back whichever message sits just before it. The
/// near miss must never be returned as though it were the message asked for —
/// the guest falls back to the thread lookup and, when that also misses,
/// reports `messaging.unknown_message`. The fallback pinches the range to
/// exactly the target ts (`oldest=latest=ts&inclusive=true`) so a reply is
/// found at any thread depth without scanning pages.
#[tokio::test]
async fn slack_get_message_returns_unknown_message_for_a_near_miss_instead_of_a_neighbour() {
    let (failure, network) = dispatch_slack_standard_op_expecting_failure(
        "slack.get_message",
        slack_user_scopes(),
        vec![
            (
                "conversations.history",
                200,
                r#"{"ok":true,"messages":[{"type":"message","user":"U0AAA","text":"neighbour","ts":"1751970001.000999"}]}"#,
            ),
            (
                "conversations.replies",
                200,
                r#"{"ok":false,"error":"thread_not_found"}"#,
            ),
        ],
        json!({"message_ref": {"conversation": "C0GENERAL", "message_id": "1751970002.000100"}}),
    )
    .await;
    assert_eq!(
        failure.kind,
        FailureKind::InputEncode,
        "an unresolvable message ref is model-fixable input: {failure:?}"
    );
    let message = failure.message.as_deref().unwrap_or_default();
    assert!(
        message.contains("messaging.unknown_message"),
        "a near miss surfaces the standard miss code, got: {message:?}"
    );
    assert!(
        !message.contains("thread_not_found"),
        "the raw vendor code must never leak: {message:?}"
    );

    let urls: Vec<String> = network.requests().iter().map(|r| r.url.clone()).collect();
    assert!(
        urls.iter().any(|u| u.contains("conversations.history")),
        "the history leg runs first: {urls:?}"
    );
    let replies_url = urls
        .iter()
        .find(|u| u.contains("conversations.replies"))
        .expect("a history miss falls back to the thread lookup");
    assert!(
        replies_url.contains("oldest=1751970002.000100")
            && replies_url.contains("latest=1751970002.000100")
            && replies_url.contains("inclusive=true"),
        "the fallback pinches the range to exactly the target ts instead of \
         scanning a fixed page: {replies_url}"
    );
}

/// A threaded REPLY is not on the conversation timeline: the history leg
/// misses and the thread fallback finds the exact ts. Only that exact match
/// is returned — enriched like every other read.
#[tokio::test]
async fn slack_get_message_falls_back_to_thread_replies_for_a_threaded_reply() {
    use ironclaw_host_api::messaging::StandardMessagingOp;
    use ironclaw_host_api::test_support::messaging_conformance::assert_canonical_output;

    let capability_id = CapabilityId::new("slack.get_message").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![
        (
            "conversations.history",
            200,
            r#"{"ok":true,"messages":[{"type":"message","user":"U0AAA","text":"thread root","ts":"1751970001.000100"}]}"#,
        ),
        (
            "conversations.replies",
            200,
            r#"{"ok":true,"messages":[{"type":"message","user":"U0AAA","text":"deep reply","ts":"1751970005.000500","thread_ts":"1751970001.000100"}]}"#,
        ),
        ("users.info?user=U0AAA", 200, SLACK_USER_AAA_BODY),
        ("auth.test", 200, SLACK_AUTH_TEST_SELF_AAA_BODY),
    ]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(network.clone(), Arc::clone(&secret_store));
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id,
            scope,
            json!({"message_ref": {"conversation": "C0GENERAL", "message_id": "1751970005.000500"}}),
        ))
        .await
        .unwrap();

    let output = match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => completed.output,
        other => panic!("expected completed outcome, got {other:?}"),
    };
    assert_canonical_output(StandardMessagingOp::GetMessage, &output);
    assert_eq!(
        output["message"]["message_ref"]["message_id"],
        json!("1751970005.000500"),
        "the exact requested reply is returned, never a neighbour: {output}"
    );
}

/// Inputs the guest must reject before Slack is ever asked: a blank
/// resolve_user query, an emoji that normalizes to nothing, and — the 1:1-DM
/// contract — an open_dm `user_ref` that is not a single well-formed Slack
/// user id. Slack's `conversations.open` takes a comma-separated LIST in its
/// `users` field, so an unvalidated "U1,U2" would silently open a group DM
/// while every contract layer promises the DM with one person. Zero egress in
/// every case, mirroring the garbled-cursor sibling above.
#[tokio::test]
async fn slack_invalid_inputs_are_rejected_before_any_vendor_call() {
    let (failure, network) = dispatch_slack_standard_op_expecting_failure(
        "slack.resolve_user",
        slack_user_scopes(),
        vec![],
        json!({"query": "   "}),
    )
    .await;
    assert_eq!(failure.kind, FailureKind::InputEncode, "{failure:?}");
    let message = failure.message.as_deref().unwrap_or_default();
    assert!(
        message.contains("messaging.unsupported_content"),
        "a blank query is unsupported content: {message:?}"
    );
    assert!(network.requests().is_empty(), "a blank query scans nothing");

    for capability in ["slack.add_reaction", "slack.remove_reaction"] {
        let (failure, network) = dispatch_slack_standard_op_expecting_failure(
            capability,
            slack_user_scopes_plus(&["reactions:read", "reactions:write"]),
            vec![],
            json!({"message_ref": general_message_ref(), "emoji": ":::"}),
        )
        .await;
        assert_eq!(
            failure.kind,
            FailureKind::InputEncode,
            "{capability}: {failure:?}"
        );
        let message = failure.message.as_deref().unwrap_or_default();
        assert!(
            message.contains("messaging.unsupported_content"),
            "{capability}: an emoji that normalizes to nothing is rejected, got: {message:?}"
        );
        assert!(
            network.requests().is_empty(),
            "{capability}: no vendor call for an empty emoji"
        );
    }

    for bad_user_ref in ["U0AAA,U0ATTACKER", "u0aaa", "C0GENERAL"] {
        let (failure, network) = dispatch_slack_standard_op_expecting_failure(
            "slack.open_dm",
            slack_user_scopes_plus(&["im:write"]),
            vec![],
            json!({"user_ref": bad_user_ref}),
        )
        .await;
        assert_eq!(
            failure.kind,
            FailureKind::InputEncode,
            "{bad_user_ref}: {failure:?}"
        );
        let message = failure.message.as_deref().unwrap_or_default();
        assert!(
            message.contains("messaging.unknown_user"),
            "{bad_user_ref}: a malformed user id is rejected as unknown_user, got: {message:?}"
        );
        assert!(
            network.requests().is_empty(),
            "{bad_user_ref}: the malformed id must never reach conversations.open"
        );
    }
}

/// Provider evidence is never fabricated: a write whose response omits the
/// evidence the canonical output requires fails as a vendor anomaly rather
/// than returning a ref with an empty id (`tool-evidence.md`: cover missing
/// evidence and malformed provider responses).
#[tokio::test]
async fn slack_write_ops_without_provider_evidence_fail_instead_of_fabricating_refs() {
    let (failure, _network) = dispatch_slack_standard_op_expecting_failure(
        "slack.open_dm",
        slack_user_scopes_plus(&["im:write"]),
        vec![("conversations.open", 200, r#"{"ok":true,"channel":{}}"#)],
        json!({"user_ref": "U0BBB"}),
    )
    .await;
    assert_eq!(failure.kind, FailureKind::OperationFailed, "{failure:?}");
    let message = failure.message.as_deref().unwrap_or_default();
    assert!(
        message.contains("messaging.vendor_error"),
        "ok:true without a conversation id is a vendor anomaly: {message:?}"
    );

    let (failure, _network) = dispatch_slack_standard_op_expecting_failure(
        "slack.edit_message",
        slack_user_write_scopes(),
        vec![("chat.update", 200, r#"{"ok":true,"channel":"C0GENERAL"}"#)],
        json!({
            "message_ref": general_message_ref(),
            "text": "edited body"
        }),
    )
    .await;
    assert_eq!(failure.kind, FailureKind::OperationFailed, "{failure:?}");
    let message = failure.message.as_deref().unwrap_or_default();
    assert!(
        message.contains("messaging.vendor_error"),
        "ok:true without a ts never becomes a message_ref with an empty id: {message:?}"
    );
}

/// `list_members` clamps its page size to Slack's real maximum (999 —
/// Slack rejects 1000), mirroring the history/list_conversations clamp pins.
/// The canonical schema caps `limit` at 1000, so 1000 is the largest value
/// that can reach the guest at all.
#[tokio::test]
async fn slack_list_members_limit_is_clamped_to_slack_maximum() {
    let capability_id = CapabilityId::new("slack.list_members").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![
        (
            "conversations.members",
            200,
            r#"{"ok":true,"members":["U0AAA"],"response_metadata":{"next_cursor":""}}"#,
        ),
        ("users.info?user=U0AAA", 200, SLACK_USER_AAA_BODY),
    ]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(network.clone(), Arc::clone(&secret_store));
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id,
            scope,
            json!({"conversation": "C0GENERAL", "limit": 1000}),
        ))
        .await
        .unwrap();
    match outcome {
        RuntimeCapabilityOutcome::Completed(_) => {}
        other => panic!("expected completed outcome, got {other:?}"),
    }

    let members_url = network
        .requests()
        .iter()
        .find(|r| r.url.contains("conversations.members"))
        .expect("conversations.members was called")
        .url
        .clone();
    assert!(
        members_url.contains("limit=999"),
        "limit 1000 must be clamped to Slack's real maximum: {members_url}"
    );
}

/// `resolve_user` scans exactly the page it was asked for: the requested
/// `users.list` page size IS the match cap, so the returned `next_cursor`
/// (page-granular on Slack) can never skip a match this call withheld — the
/// regression where a mid-page break dropped matches between the cap and the
/// cursor. Default (no `limit`) scans the full 200-entry page.
#[tokio::test]
async fn slack_resolve_user_scans_exactly_the_requested_page_so_cursors_never_skip_matches() {
    // Default: the full 200-entry page is requested.
    let default_capability_id = CapabilityId::new("slack.resolve_user").unwrap();
    let default_scope = sample_scope(InvocationId::new());
    let default_network = UrlKeyedSlackEgress::new(vec![(
        "users.list",
        200,
        r#"{"ok":true,"members":[{"id":"U0BBB","name":"bob","profile":{"real_name":"Bob Example","display_name":"Bob"}}],"response_metadata":{"next_cursor":""}}"#,
    )]);
    let default_secret_store = Arc::new(SecretStore::ephemeral());
    let default_services = slack_enrichment_services_for_test!(
        default_network.clone(),
        Arc::clone(&default_secret_store)
    );
    seed_slack_user_token(&default_secret_store, &default_scope).await;
    let default_outcome = default_services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            default_capability_id,
            default_scope,
            json!({"query": "bob"}),
        ))
        .await
        .unwrap();
    match default_outcome {
        RuntimeCapabilityOutcome::Completed(_) => {}
        other => panic!("expected completed outcome, got {other:?}"),
    }
    let default_url = default_network
        .requests()
        .iter()
        .find(|r| r.url.contains("users.list"))
        .expect("users.list was called")
        .url
        .clone();
    assert!(
        default_url.contains("limit=200"),
        "an omitted limit scans the full 200-entry page: {default_url}"
    );

    // Explicit limit: page == limit, and even a page Slack overshoots never
    // yields more than `limit` matches; the cursor is passed through so the
    // caller resumes at the NEXT page with nothing withheld in between.
    let capability_id = CapabilityId::new("slack.resolve_user").unwrap();
    let scope = sample_scope(InvocationId::new());
    let network = UrlKeyedSlackEgress::new(vec![(
        "users.list",
        200,
        r#"{"ok":true,"members":[
            {"id":"U0AAA","name":"bob-one","profile":{"real_name":"Bob One","display_name":"Bob One"}},
            {"id":"U0BBB","name":"bob-two","profile":{"real_name":"Bob Two","display_name":"Bob Two"}},
            {"id":"U0CCC","name":"bob-three","profile":{"real_name":"Bob Three","display_name":"Bob Three"}}
        ],"response_metadata":{"next_cursor":"cur9"}}"#,
    )]);
    let secret_store = Arc::new(SecretStore::ephemeral());
    let services = slack_enrichment_services_for_test!(network.clone(), Arc::clone(&secret_store));
    seed_slack_user_token(&secret_store, &scope).await;

    let outcome = services
        .host_runtime_for_local_testing()
        .invoke_capability(wasm_runtime_request_for_scope(
            capability_id,
            scope,
            json!({"query": "bob", "limit": 2}),
        ))
        .await
        .unwrap();
    let output = match outcome {
        RuntimeCapabilityOutcome::Completed(completed) => completed.output,
        other => panic!("expected completed outcome, got {other:?}"),
    };
    assert_eq!(
        output["matches"].as_array().map(Vec::len),
        Some(2),
        "an overshot page never yields more than `limit` matches: {output}"
    );
    assert_eq!(
        output["next_cursor"],
        json!("cur9"),
        "the page cursor is passed through for loss-free resumption: {output}"
    );

    let list_url = network
        .requests()
        .iter()
        .find(|r| r.url.contains("users.list"))
        .expect("users.list was called")
        .url
        .clone();
    assert!(
        list_url.contains("limit=2"),
        "the scanned page IS the requested limit: {list_url}"
    );
}

/// An empty conversation is an honest empty result — `members: []`, no
/// cursor — never an error and never a fabricated member.
#[tokio::test]
async fn slack_list_members_reports_an_empty_conversation_honestly() {
    use ironclaw_host_api::messaging::StandardMessagingOp;
    use ironclaw_host_api::test_support::messaging_conformance::assert_canonical_output;

    let output = dispatch_slack_standard_op_for_conformance(
        "slack.list_members",
        slack_user_scopes(),
        vec![(
            "conversations.members",
            200,
            r#"{"ok":true,"members":[],"response_metadata":{"next_cursor":""}}"#,
        )],
        json!({"conversation": "C0QUIET"}),
    )
    .await;
    assert_canonical_output(StandardMessagingOp::ListMembers, &output);
    assert_eq!(
        output["members"],
        json!([]),
        "an empty conversation reads back as an empty member list: {output}"
    );
    assert!(
        output.get("next_cursor").is_none(),
        "no cursor is reported when Slack signals the last page: {output}"
    );
}
