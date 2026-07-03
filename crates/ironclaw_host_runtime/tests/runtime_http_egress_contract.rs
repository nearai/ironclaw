use ironclaw_capabilities::{
    CapabilityObligationHandler, CapabilityObligationPhase, CapabilityObligationRequest,
};
use ironclaw_events::InMemoryAuditSink;
use ironclaw_filesystem::{InMemoryBackend, LocalFilesystem, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    AgentId, CapabilityId, CapabilitySet, CredentialStageError, ExecutionContext, ExtensionId,
    InvocationId, MountAlias, MountGrant, MountPermissions, MountView, NetworkMethod,
    NetworkPolicy, NetworkScheme, NetworkTargetPattern, Obligation, ProjectId, ResourceEstimate,
    ResourceScope, RuntimeCredentialAccountProviderId, RuntimeCredentialInjection,
    RuntimeCredentialSource, RuntimeCredentialTarget, RuntimeHttpEgress, RuntimeHttpEgressError,
    RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, RuntimeHttpSaveTarget, RuntimeKind,
    ScopedPath, SecretHandle, TenantId, TrustClass, UserId, VirtualPath,
};
use ironclaw_host_runtime::{
    BuiltinObligationServices, RuntimeCredentialAccessSecret, RuntimeCredentialAccountRequest,
    RuntimeCredentialAccountResolver, RuntimeHttpBodyStore, RuntimeHttpBodyStoreError,
    ToolCallHttpEgress,
};
use ironclaw_mcp::{
    McpClient, McpClientRequest, McpHostHttpClient, McpHostHttpEgressPlan, McpHostHttpRequest,
    McpRuntimeHttpAdapter, StaticMcpHostHttpEgressPlanner,
};
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
    PolicyNetworkHttpEgress, ReqwestNetworkTransport,
};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_scripts::{ScriptHostHttpRequest, ScriptRuntimeHttpAdapter};
use ironclaw_secrets::{InMemorySecretStore, SecretMaterial, SecretStore};
use ironclaw_wasm::{
    WasmHostHttp, WasmHttpRequest, WasmRuntimeHttpAdapter, WasmStagedRuntimeCredential,
    WasmStagedRuntimeCredentials,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    time::Duration,
};
use tempfile::tempdir;

#[test]
fn tool_call_http_egress_returns_sanitized_partial_response_for_model_visible_output() {
    let network = RecordingNetwork::err(NetworkHttpError::ResponseBodyLimit {
        limit: 4,
        request_bytes: 0,
        response_bytes: 5,
        partial_response: Some(NetworkHttpResponse {
            status: 200,
            headers: vec![
                (
                    "authorization".to_string(),
                    "Bearer sk-response-secret".to_string(),
                ),
                ("x-safe".to_string(), "visible".to_string()),
            ],
            body: b"abcd".to_vec(),
            usage: NetworkUsage {
                request_bytes: 0,
                response_bytes: 5,
                resolved_ip: None,
            },
        }),
    });
    let scope = sample_scope();
    let capability_id = CapabilityId::new("builtin.http").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    let service = services.host_http_egress(network);

    let response = block_on_test(service.execute_for_model_visible_output(
        RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope,
            capability_id,
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/items".to_string(),
            headers: vec![],
            body: Vec::new(),
            network_policy: sample_policy(),
            credential_injections: Vec::new(),
            response_body_limit: Some(4),
            save_body_to: None,
            timeout_ms: None,
        },
    ))
    .expect("tool-call port should receive sanitized partial response");

    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"abcd");
    assert_eq!(response.response_bytes, 5);
    assert_eq!(
        response.headers,
        vec![("x-safe".to_string(), "visible".to_string())]
    );
    assert!(response.redaction_applied);
}

#[test]
fn runtime_http_egress_keeps_partial_response_limit_strict() {
    let network = RecordingNetwork::err(NetworkHttpError::ResponseBodyLimit {
        limit: 4,
        request_bytes: 0,
        response_bytes: 5,
        partial_response: Some(NetworkHttpResponse {
            status: 200,
            headers: vec![],
            body: b"abcd".to_vec(),
            usage: NetworkUsage {
                request_bytes: 0,
                response_bytes: 5,
                resolved_ip: None,
            },
        }),
    });
    let scope = sample_scope();
    let capability_id = CapabilityId::new("builtin.http").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    let service = services.host_http_egress(network);

    let error = block_on_test(service.execute(RuntimeHttpEgressRequest {
        runtime: RuntimeKind::FirstParty,
        scope,
        capability_id,
        method: NetworkMethod::Get,
        url: "https://api.example.test/v1/items".to_string(),
        headers: vec![],
        body: Vec::new(),
        network_policy: sample_policy(),
        credential_injections: Vec::new(),
        response_body_limit: Some(4),
        save_body_to: None,
        timeout_ms: None,
    }))
    .expect_err("generic runtime HTTP egress should keep response limits strict");

    assert!(matches!(error, RuntimeHttpEgressError::Network { .. }));
    assert_eq!(
        error.stable_runtime_reason(),
        "response_body_limit_exceeded"
    );
    assert_eq!(error.response_bytes(), 5);
}

#[test]
fn tool_call_http_egress_returns_network_error_when_partial_response_is_missing() {
    let network = RecordingNetwork::err(NetworkHttpError::ResponseBodyLimit {
        limit: 4,
        request_bytes: 0,
        response_bytes: 5,
        partial_response: None,
    });
    let scope = sample_scope();
    let capability_id = CapabilityId::new("builtin.http").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    let service = services.host_http_egress(network);

    let error = block_on_test(
        service.execute_for_model_visible_output(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope,
            capability_id,
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/items".to_string(),
            headers: vec![],
            body: Vec::new(),
            network_policy: sample_policy(),
            credential_injections: Vec::new(),
            response_body_limit: Some(4),
            save_body_to: None,
            timeout_ms: None,
        }),
    )
    .expect_err("missing partial response should keep response limits strict");

    assert!(matches!(error, RuntimeHttpEgressError::Network { .. }));
    assert_eq!(
        error.stable_runtime_reason(),
        "response_body_limit_exceeded"
    );
    assert_eq!(error.response_bytes(), 5);
}

#[tokio::test]
async fn host_http_egress_consumes_staged_obligation_secret_once() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    stage_secret_sync(
        &services,
        &scope,
        &capability_id,
        &handle,
        "sk-staged-secret",
    );
    let service = services.host_http_egress(network);

    let request = RuntimeHttpEgressRequest {
        runtime: RuntimeKind::Script,
        scope: scope.clone(),
        capability_id: sample_capability_id(),
        method: NetworkMethod::Post,
        url: "https://api.example.test/v1/run".to_string(),
        headers: vec![],
        body: b"hello".to_vec(),
        network_policy: sample_policy(),
        credential_injections: vec![RuntimeCredentialInjection {
            handle: handle.clone(),
            source: RuntimeCredentialSource::StagedObligation {
                capability_id: capability_id.clone(),
            },
            target: RuntimeCredentialTarget::Header {
                name: "authorization".to_string(),
                prefix: Some("Bearer ".to_string()),
            },
            required: true,
        }],
        response_body_limit: Some(4096),
        save_body_to: None,
        timeout_ms: None,
    };

    service
        .execute(request.clone())
        .await
        .expect("staged secret should be injected through host egress");

    {
        let requests = network_recorder.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0]
                .headers
                .iter()
                .find(|(name, _)| name == "authorization"),
            Some(&(
                "authorization".to_string(),
                "Bearer sk-staged-secret".to_string()
            ))
        );
    }

    let error = service
        .execute(request)
        .await
        .expect_err("staged secret must not be reusable");
    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Credential { .. }
    ));
    assert_eq!(network_recorder.lock().unwrap().len(), 1);
}

#[test]
fn host_http_egress_records_injected_credentials_in_zeroizing_network_request() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    stage_secret_sync(
        &services,
        &scope,
        &capability_id,
        &handle,
        "sk-staged-secret",
    );
    let service = services.host_http_egress(network);

    block_on_test(service.execute(RuntimeHttpEgressRequest {
        runtime: RuntimeKind::Script,
        scope,
        capability_id: capability_id.clone(),
        method: NetworkMethod::Post,
        url: "https://api.example.test/v1/run".to_string(),
        headers: vec![],
        body: b"hello".to_vec(),
        network_policy: sample_policy(),
        credential_injections: vec![RuntimeCredentialInjection {
            handle,
            source: RuntimeCredentialSource::StagedObligation { capability_id },
            target: RuntimeCredentialTarget::Header {
                name: "authorization".to_string(),
                prefix: Some("Bearer ".to_string()),
            },
            required: true,
        }],
        response_body_limit: Some(4096),
        save_body_to: None,
        timeout_ms: None,
    }))
    .expect("staged secret should be injected through host egress");

    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    require_zeroize_on_drop(&requests[0]);
    assert_eq!(
        requests[0]
            .headers
            .iter()
            .find(|(name, _)| name == "authorization"),
        Some(&(
            "authorization".to_string(),
            "Bearer sk-staged-secret".to_string()
        ))
    );
}

fn require_zeroize_on_drop<T: ?Sized + zeroize::ZeroizeOnDrop>(_: &T) {}

#[tokio::test]
async fn host_http_egress_consumes_secret_staged_by_builtin_obligation_handler() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let secret_store = Arc::new(InMemorySecretStore::new());
    let services = BuiltinObligationServices::new(
        Arc::new(InMemoryAuditSink::new()),
        secret_store.clone(),
        Arc::new(InMemoryResourceGovernor::new()),
    );
    let handler = services.obligation_handler();
    let service = services.host_http_egress(network);
    let context = execution_context();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    secret_store
        .put(
            context.resource_scope.clone(),
            handle.clone(),
            SecretMaterial::from("sk-staged-secret"),
            None,
        )
        .await
        .unwrap();
    let obligations = vec![
        Obligation::ApplyNetworkPolicy {
            policy: sample_policy(),
        },
        Obligation::InjectSecretOnce {
            handle: handle.clone(),
        },
    ];

    handler
        .satisfy(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &ResourceEstimate::default(),
            obligations: &obligations,
        })
        .await
        .expect("obligation handler should stage secret material");

    service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: context.resource_scope.clone(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect("host egress should consume material staged by the obligation handler");

    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]
            .headers
            .iter()
            .find(|(name, _)| name == "authorization"),
        Some(&(
            "authorization".to_string(),
            "Bearer sk-staged-secret".to_string()
        ))
    );
}

#[tokio::test]
async fn host_http_egress_reuses_staged_secret_for_multiple_targets_in_one_request() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{\"ok\":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    stage_secret_sync(
        &services,
        &scope,
        &capability_id,
        &handle,
        "sk-staged-secret",
    );
    let service = services.host_http_egress(network);

    service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: scope.clone(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/__credential__/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![
                RuntimeCredentialInjection {
                    handle: handle.clone(),
                    source: RuntimeCredentialSource::StagedObligation {
                        capability_id: capability_id.clone(),
                    },
                    target: RuntimeCredentialTarget::Header {
                        name: "authorization".to_string(),
                        prefix: Some("Bearer ".to_string()),
                    },
                    required: true,
                },
                RuntimeCredentialInjection {
                    handle: handle.clone(),
                    source: RuntimeCredentialSource::StagedObligation {
                        capability_id: capability_id.clone(),
                    },
                    target: RuntimeCredentialTarget::QueryParam {
                        name: "token".to_string(),
                    },
                    required: true,
                },
                RuntimeCredentialInjection {
                    handle: handle.clone(),
                    source: RuntimeCredentialSource::StagedObligation {
                        capability_id: capability_id.clone(),
                    },
                    target: RuntimeCredentialTarget::PathPlaceholder {
                        placeholder: "__credential__".to_string(),
                    },
                    required: true,
                },
            ],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect("same staged handle should be reusable within a single request plan");

    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]
            .headers
            .iter()
            .find(|(name, _)| name == "authorization"),
        Some(&(
            "authorization".to_string(),
            "Bearer sk-staged-secret".to_string()
        ))
    );
    assert_eq!(
        requests[0].url,
        "https://api.example.test/v1/sk-staged-secret/run?token=sk-staged-secret"
    );
    drop(requests);
}

#[tokio::test]
async fn host_http_egress_restores_staged_secret_when_later_injection_target_fails() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{\"ok\":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    stage_secret_sync(
        &services,
        &scope,
        &capability_id,
        &handle,
        "sk-staged-secret",
    );
    let service = services.host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: scope.clone(),
            capability_id: capability_id.clone(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/__credential__/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![
                RuntimeCredentialInjection {
                    handle: handle.clone(),
                    source: RuntimeCredentialSource::StagedObligation {
                        capability_id: capability_id.clone(),
                    },
                    target: RuntimeCredentialTarget::Header {
                        name: "authorization".to_string(),
                        prefix: Some("Bearer ".to_string()),
                    },
                    required: true,
                },
                RuntimeCredentialInjection {
                    handle: handle.clone(),
                    source: RuntimeCredentialSource::StagedObligation {
                        capability_id: capability_id.clone(),
                    },
                    target: RuntimeCredentialTarget::PathPlaceholder {
                        placeholder: "__missing__".to_string(),
                    },
                    required: true,
                },
            ],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("later injection target failure should fail before dispatch");

    assert!(matches!(
        error,
        RuntimeHttpEgressError::Credential { ref reason }
            if reason == "credential injection path placeholder was not found"
    ));
    assert!(network_recorder.lock().unwrap().is_empty());

    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: scope.clone(),
            capability_id: capability_id.clone(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle: handle.clone(),
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect("staged secret should be restored after target-application failure");

    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]
            .headers
            .iter()
            .find(|(name, _)| name == "authorization"),
        Some(&(
            "authorization".to_string(),
            "Bearer sk-staged-secret".to_string()
        ))
    );
}

#[tokio::test]
async fn host_http_egress_rejects_invalid_path_placeholder_before_transport() {
    for (placeholder, url) in [
        ("", "https://api.example.test/v1/__credential__/run"),
        (
            "bad/placeholder",
            "https://api.example.test/v1/__credential__/run",
        ),
        (
            "bad?placeholder",
            "https://api.example.test/v1/__credential__/run",
        ),
        (
            "bad#placeholder",
            "https://api.example.test/v1/__credential__/run",
        ),
        (
            "__missing__",
            "https://api.example.test/v1/__credential__/run",
        ),
    ] {
        let network = RecordingNetwork::ok(NetworkHttpResponse {
            status: 200,
            headers: vec![],
            body: br#"{"ok":true}"#.to_vec(),
            usage: NetworkUsage {
                request_bytes: 5,
                response_bytes: 11,
                resolved_ip: None,
            },
        });
        let network_recorder = network.requests.clone();
        let scope = sample_scope();
        let capability_id = sample_capability_id();
        let handle = SecretHandle::new("api-token").unwrap();
        let services = test_obligation_services();
        stage_policy_sync(&services, &scope, &capability_id, sample_policy());
        stage_secret_sync(
            &services,
            &scope,
            &capability_id,
            &handle,
            "sk-staged-secret",
        );
        let service = services.host_http_egress(network);

        let error = service
            .execute(RuntimeHttpEgressRequest {
                runtime: RuntimeKind::Script,
                scope: scope.clone(),
                capability_id: capability_id.clone(),
                method: NetworkMethod::Post,
                url: url.to_string(),
                headers: vec![],
                body: b"hello".to_vec(),
                network_policy: sample_policy(),
                credential_injections: vec![RuntimeCredentialInjection {
                    handle: handle.clone(),
                    source: RuntimeCredentialSource::StagedObligation {
                        capability_id: capability_id.clone(),
                    },
                    target: RuntimeCredentialTarget::PathPlaceholder {
                        placeholder: placeholder.to_string(),
                    },
                    required: true,
                }],
                response_body_limit: Some(4096),
                save_body_to: None,
                timeout_ms: None,
            })
            .await
            .expect_err("invalid path placeholder must fail before network dispatch");

        assert!(matches!(
            error,
            ironclaw_host_api::RuntimeHttpEgressError::Credential { .. }
        ));
        assert!(
            network_recorder.lock().unwrap().is_empty(),
            "case {placeholder:?} must not dispatch to the network"
        );
    }
}

#[tokio::test]
async fn host_http_egress_rejects_path_placeholder_value_breaking_chars_before_transport() {
    for material in [
        "",
        ".",
        "..",
        "sk-staged/secret",
        "sk-staged?secret",
        "sk-staged#secret",
        "sk-staged\nsecret",
        "sk-staged\0secret",
        "sk-staged+secret",
    ] {
        let (error, network_recorder) = execute_path_placeholder_egress(
            "https://api.example.test/v1/__credential__/run",
            "__credential__",
            material,
        )
        .await
        .expect_err("invalid path placeholder credential value must fail before network dispatch");

        assert!(matches!(
            error,
            ironclaw_host_api::RuntimeHttpEgressError::Credential { .. }
        ));
        assert!(error.to_string().contains("path value is invalid"));
        assert!(
            network_recorder.lock().unwrap().is_empty(),
            "material {material:?} must not dispatch to the network"
        );
    }
}

#[tokio::test]
async fn host_http_egress_requires_https_for_path_placeholder_before_transport() {
    let (error, network_recorder) = execute_path_placeholder_egress(
        "http://api.example.test/v1/__credential__/run",
        "__credential__",
        "sk-staged-secret",
    )
    .await
    .expect_err("path placeholder credential injection must require HTTPS");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Credential { .. }
    ));
    assert!(error.to_string().contains("requires HTTPS"));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_rejects_multiple_path_placeholder_occurrences_before_transport() {
    let (error, network_recorder) = execute_path_placeholder_egress(
        "https://api.example.test/__credential__/v1/__credential__/run",
        "__credential__",
        "sk-staged-secret",
    )
    .await
    .expect_err("path placeholder credential injection must have one target segment");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Credential { .. }
    ));
    assert!(error.to_string().contains("exactly once"));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_preserves_existing_path_encoding_when_rewriting_placeholder() {
    let (_response, network_recorder) = execute_path_placeholder_egress(
        "https://api.example.test/v1/foo%20bar/__credential__/run%2Ftail",
        "__credential__",
        "sk-staged-secret",
    )
    .await
    .expect("path placeholder rewrite should preserve existing encoded segments");

    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url,
        "https://api.example.test/v1/foo%20bar/sk-staged-secret/run%2Ftail"
    );
}

#[tokio::test]
async fn host_http_egress_rejects_path_placeholder_target_url_errors_before_transport() {
    for (url, expected_reason) in [
        ("not a url", "credential injection target URL is invalid"),
        (
            "mailto:security@example.test",
            "credential injection path placeholder requires HTTPS",
        ),
    ] {
        let (error, network_recorder) =
            execute_path_placeholder_egress(url, "__credential__", "sk-staged-secret")
                .await
                .expect_err(
                    "invalid path placeholder target URL must fail before network dispatch",
                );

        assert!(matches!(
            error,
            ironclaw_host_api::RuntimeHttpEgressError::Credential { .. }
        ));
        assert!(error.to_string().contains(expected_reason));
        assert!(
            network_recorder.lock().unwrap().is_empty(),
            "url {url:?} must not dispatch to the network"
        );
    }
}

#[tokio::test]
async fn host_http_egress_fails_closed_when_required_staged_secret_is_missing() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    let service = services.host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: scope.clone(),
            capability_id: capability_id.clone(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle: SecretHandle::new("api-token").unwrap(),
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("missing staged material must fail before network dispatch");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Credential { .. }
    ));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_does_not_take_staged_secret_from_other_capability() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let scope = sample_scope();
    let requested_capability = sample_capability_id();
    let other_capability = CapabilityId::new("other.capability").unwrap();
    let handle = SecretHandle::new("api-token").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &requested_capability, sample_policy());
    stage_secret_sync(
        &services,
        &scope,
        &other_capability,
        &handle,
        "sk-staged-secret",
    );
    let service = services.host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: scope.clone(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle: handle.clone(),
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: requested_capability,
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("staged material for a different capability must not authorize egress");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Credential { .. }
    ));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_does_not_take_staged_secret_for_other_handle() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let requested_handle = SecretHandle::new("api-token").unwrap();
    let other_handle = SecretHandle::new("other-token").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    stage_secret_sync(
        &services,
        &scope,
        &capability_id,
        &other_handle,
        "sk-staged-secret",
    );
    let service = services.host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: scope.clone(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle: requested_handle,
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("staged material for a different handle must not authorize egress");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Credential { .. }
    ));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_removes_staged_secret_before_network_errors() {
    let network = RecordingNetwork::err(NetworkHttpError::Transport {
        reason: "upstream rejected sk-staged-secret".to_string(),
        request_bytes: 12,
        response_bytes: 0,
    });
    let network_recorder = network.requests.clone();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    stage_secret_sync(
        &services,
        &scope,
        &capability_id,
        &handle,
        "sk-staged-secret",
    );
    let service = services.host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: scope.clone(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle: handle.clone(),
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("network error should be sanitized after staged injection is consumed");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Network { .. }
    ));
    assert!(!error.to_string().contains("sk-staged-secret"));
    assert_eq!(network_recorder.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn host_http_egress_skips_optional_missing_staged_secret() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    let service = services.host_http_egress(network);

    let response = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: scope.clone(),
            capability_id: capability_id.clone(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle: SecretHandle::new("api-token").unwrap(),
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: false,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect("optional missing staged material should not block egress");

    assert_eq!(response.status, 200);
    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .headers
            .iter()
            .all(|(name, _)| name != "authorization"),
        "optional missing staged material should not inject a credential"
    );
}

#[tokio::test]
async fn host_http_egress_does_not_take_staged_secret_from_other_scope() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let requested_scope = sample_scope();
    let other_scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &requested_scope, &capability_id, sample_policy());
    stage_secret_sync(
        &services,
        &other_scope,
        &capability_id,
        &handle,
        "sk-staged-secret",
    );
    let service = services.host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: requested_scope,
            capability_id: sample_capability_id(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle: handle.clone(),
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("staged material for a different scope must not authorize egress");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Credential { .. }
    ));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_rejects_header_injection_prefix_control_chars() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    stage_secret_sync(&services, &scope, &capability_id, &handle, "sk-test-secret");
    let service = services.host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope,
            capability_id: capability_id.clone(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer \r\nx-evil: ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("header injection prefixes with control characters must be rejected");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Credential { .. }
    ));
    assert!(!error.to_string().contains("sk-test-secret"));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_rejects_header_injection_value_control_chars() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    stage_secret_sync(
        &services,
        &scope,
        &capability_id,
        &handle,
        "sk-test-secret\r\nx-evil: injected",
    );
    let service = services.host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope,
            capability_id: capability_id.clone(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: None,
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("header injection values with control characters must be rejected");

    assert_eq!(
        credential_reason(&error),
        "credential injection header value is invalid"
    );
    assert!(!error.to_string().contains("sk-test-secret"));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_injects_staged_credentials_and_redacts_errors() {
    let network = RecordingNetwork::err(NetworkHttpError::Transport {
        reason: "upstream rejected token sk-test-secret".to_string(),
        request_bytes: 12,
        response_bytes: 0,
    });
    let network_recorder = network.requests.clone();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    stage_secret_sync(&services, &scope, &capability_id, &handle, "sk-test-secret");
    let service = services.host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope,
            capability_id: capability_id.clone(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("network error should be sanitized");

    let rendered = error.to_string();
    assert!(rendered.contains("transport_failed"));
    assert!(!rendered.contains("sk-test-secret"));
    assert_eq!(error.request_bytes(), 12);
    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]
            .headers
            .iter()
            .find(|(name, _)| name == "authorization"),
        Some(&(
            "authorization".to_string(),
            "Bearer sk-test-secret".to_string()
        ))
    );
}

#[tokio::test]
async fn host_http_egress_requires_available_required_credentials_before_network() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    let service = services.host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope,
            capability_id: capability_id.clone(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle: SecretHandle::new("missing-token").unwrap(),
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("missing required credentials should fail before network dispatch");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Credential { .. }
    ));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_required_credential_still_fails_after_optional_negative_cache() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let handle = SecretHandle::new("missing-token").unwrap();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    let service = services.host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope,
            capability_id: capability_id.clone(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![
                RuntimeCredentialInjection {
                    handle: handle.clone(),
                    source: RuntimeCredentialSource::StagedObligation {
                        capability_id: capability_id.clone(),
                    },
                    target: RuntimeCredentialTarget::Header {
                        name: "x-optional-token".to_string(),
                        prefix: Some("Bearer ".to_string()),
                    },
                    required: false,
                },
                RuntimeCredentialInjection {
                    handle,
                    source: RuntimeCredentialSource::StagedObligation {
                        capability_id: capability_id.clone(),
                    },
                    target: RuntimeCredentialTarget::Header {
                        name: "authorization".to_string(),
                        prefix: Some("Bearer ".to_string()),
                    },
                    required: true,
                },
            ],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("required reuse of a negatively cached credential must fail closed");

    assert_eq!(
        credential_reason(&error),
        "required credential is unavailable"
    );
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_injects_and_redacts_url_encoded_query_credentials() {
    let network = UrlEchoNetwork::new();
    let network_recorder = network.requests.clone();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    stage_secret_sync(
        &services,
        &scope,
        &capability_id,
        &handle,
        "secret with/slash+plus?",
    );
    let service = services.host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope,
            capability_id: capability_id.clone(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::QueryParam {
                    name: "token".to_string(),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("network error should be sanitized");

    let rendered = error.to_string();
    assert!(rendered.contains("transport_failed"));
    assert!(!rendered.contains("secret with/slash+plus?"));
    assert!(!rendered.contains("secret+with%2Fslash%2Bplus%3F"));
    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url,
        "https://api.example.test/v1/run?token=secret+with%2Fslash%2Bplus%3F"
    );
}

#[tokio::test]
async fn host_http_egress_redacts_path_placeholder_credentials_from_response() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 302,
        headers: vec![(
            "location".to_string(),
            "https://api.example.test/v1/sk-staged-secret/next".to_string(),
        )],
        body: b"upstream echoed sk-staged-secret".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 30,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    stage_secret_sync(
        &services,
        &scope,
        &capability_id,
        &handle,
        "sk-staged-secret",
    );
    let service = services.host_http_egress(network);

    let response = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope,
            capability_id: capability_id.clone(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/__credential__/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                source: RuntimeCredentialSource::StagedObligation { capability_id },
                target: RuntimeCredentialTarget::PathPlaceholder {
                    placeholder: "__credential__".to_string(),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect("path placeholder credential response echoes should be redacted");

    assert_eq!(response.status, 302);
    assert!(response.redaction_applied);
    let rendered_body = String::from_utf8(response.body).unwrap();
    assert!(rendered_body.contains("[REDACTED]"));
    assert!(!rendered_body.contains("sk-staged-secret"));
    let location = response
        .headers
        .iter()
        .find(|(name, _)| name == "location")
        .map(|(_, value)| value.as_str())
        .expect("location header should be preserved after redaction");
    assert!(location.contains("[REDACTED]"));
    assert!(!location.contains("sk-staged-secret"));

    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url,
        "https://api.example.test/v1/sk-staged-secret/run"
    );
}

#[tokio::test]
async fn host_http_egress_redacts_path_placeholder_credentials_from_network_errors() {
    let network = UrlEchoNetwork::new();
    let network_recorder = network.requests.clone();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    stage_secret_sync(
        &services,
        &scope,
        &capability_id,
        &handle,
        "sk-staged-secret",
    );
    let service = services.host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope,
            capability_id: capability_id.clone(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/__credential__/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                source: RuntimeCredentialSource::StagedObligation { capability_id },
                target: RuntimeCredentialTarget::PathPlaceholder {
                    placeholder: "__credential__".to_string(),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("network errors after path placeholder injection should be sanitized");

    let rendered = error.to_string();
    assert!(rendered.contains("transport_failed"));
    assert!(!rendered.contains("sk-staged-secret"));
    assert!(!rendered.contains("api.example.test/v1/sk-staged-secret/run"));
    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url,
        "https://api.example.test/v1/sk-staged-secret/run"
    );
}

#[tokio::test]
async fn host_http_egress_forwards_timeout_to_network() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{\"ok\":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let service = request_policy_staging_egress(network);

    service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Wasm,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: Some(250),
        })
        .await
        .expect("network response should be returned");

    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].timeout_ms, Some(250));
}

#[tokio::test]
async fn host_http_egress_with_reqwest_transport_returns_redirect_without_following() {
    let (url, server) = single_response_server(
        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/followed\r\nContent-Length: 0\r\n\r\n",
    );
    let network =
        PolicyNetworkHttpEgress::new(ReqwestNetworkTransport::new(Duration::from_secs(2)));
    let service = request_policy_staging_egress(network);

    let response = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Get,
            url,
            headers: vec![],
            body: Vec::new(),
            network_policy: local_http_policy(),
            credential_injections: vec![],
            response_body_limit: Some(1024),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect("redirect responses should be returned to the caller, not followed");
    server.join().unwrap();

    assert_eq!(response.status, 302);
    assert_eq!(
        response
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("location"))
            .map(|(_, value)| value.as_str()),
        Some("http://127.0.0.1:9/followed")
    );
}

#[tokio::test]
async fn host_http_egress_preserves_request_and_response_byte_accounting() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let service = request_policy_staging_egress(network);

    let response = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Mcp,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/mcp".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect("network response should be returned");

    assert_eq!(response.request_bytes, 5);
    assert_eq!(response.response_bytes, 11);
    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body, b"hello");
    assert_eq!(requests[0].response_body_limit, Some(4096));
}

#[tokio::test]
async fn host_http_egress_without_policy_store_fails_closed_before_transport() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{\"ok\":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let service = test_obligation_services().host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Wasm,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: caller_supplied_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("runtime HTTP egress must not trust caller-supplied network policy without a staged-policy store");

    assert!(matches!(
        error,
        RuntimeHttpEgressError::Network {
            reason,
            request_bytes: 0,
            response_bytes: 0,
        } if reason == "network_policy_missing"
    ));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_borrows_staged_network_policy_before_transport() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{\"ok\":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let services = test_obligation_services();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let staged_policy = sample_policy();
    stage_policy_sync(&services, &scope, &capability_id, staged_policy.clone());
    let service = services.host_http_egress(network);

    service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Wasm,
            scope: scope.clone(),
            capability_id: capability_id.clone(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: caller_supplied_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect("staged network policy should authorize host-mediated HTTP");

    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].policy, staged_policy);
    drop(requests);
}

#[tokio::test]
async fn production_host_http_egress_rejects_direct_secret_store_lease_before_transport() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{\"ok\":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let services = test_obligation_services();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    block_on_test(services.secret_store().put(
        scope.clone(),
        handle.clone(),
        SecretMaterial::from("sk-direct-lease"),
        None,
    ))
    .unwrap();
    let service = services.host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Mcp,
            scope,
            capability_id,
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: caller_supplied_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                source: RuntimeCredentialSource::SecretStoreLease,
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: Some(1000),
        })
        .await
        .expect_err("production egress must require staged secret obligations");

    assert!(matches!(
        error,
        RuntimeHttpEgressError::Credential { reason }
            if reason == "direct secret-store leases are unavailable for production runtime egress"
    ));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn production_host_http_egress_discards_staged_policy_when_direct_secret_store_lease_is_rejected()
 {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{\"ok\":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let services = test_obligation_services();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    let service = services.host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Mcp,
            scope: scope.clone(),
            capability_id: capability_id.clone(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: caller_supplied_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                source: RuntimeCredentialSource::SecretStoreLease,
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: Some(1000),
        })
        .await
        .expect_err("rejected direct lease should discard staged policy");
    assert!(matches!(error, RuntimeHttpEgressError::Credential { .. }));

    let retry = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Mcp,
            scope,
            capability_id,
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: caller_supplied_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: Some(1000),
        })
        .await
        .expect_err("discarded staged policy must not authorize retry");

    assert!(matches!(
        retry,
        RuntimeHttpEgressError::Network {
            reason,
            request_bytes: 0,
            response_bytes: 0,
        } if reason == "network_policy_missing"
    ));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn production_host_http_egress_rejects_cross_capability_staged_credentials_before_transport()
{
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{\"ok\":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let services = test_obligation_services();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let other_capability_id = CapabilityId::new("other.http").unwrap();
    let handle = SecretHandle::new("api-token").unwrap();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    stage_secret_sync(
        &services,
        &scope,
        &other_capability_id,
        &handle,
        "sk-other-capability",
    );
    let service = services.host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Mcp,
            scope,
            capability_id,
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: caller_supplied_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: other_capability_id,
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: Some(1000),
        })
        .await
        .expect_err("cross-capability staged credentials must be rejected");

    assert!(matches!(
        error,
        RuntimeHttpEgressError::Credential { reason }
            if reason == "staged credential capability does not match request capability"
    ));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn wasm_http_adapter_borrows_real_host_staged_network_policy() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: b"ok".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 2,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let services = test_obligation_services();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let staged_policy = sample_policy();
    stage_policy_sync(&services, &scope, &capability_id, staged_policy.clone());
    let service = services.host_http_egress(network);
    let adapter = WasmRuntimeHttpAdapter::new(
        Arc::new(service),
        scope.clone(),
        capability_id.clone(),
        caller_supplied_policy(),
    );

    let response = adapter
        .request(WasmHttpRequest {
            method: "POST".to_string(),
            url: "https://api.example.test/v1/run".to_string(),
            headers_json: "{}".to_string(),
            body: Some(b"hello".to_vec()),
            timeout_ms: Some(1000),
        })
        .expect("WASM adapter should reach host egress using staged policy");

    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"ok".to_vec());
    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].policy, staged_policy);
    assert_eq!(requests[0].url, "https://api.example.test/v1/run");
    assert_eq!(requests[0].body, b"hello".to_vec());
    drop(requests);
}

#[tokio::test]
async fn script_http_adapter_borrows_real_host_staged_network_policy() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 202,
        headers: vec![],
        body: b"script-ok".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 9,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let services = test_obligation_services();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let staged_policy = sample_policy();
    stage_policy_sync(&services, &scope, &capability_id, staged_policy.clone());
    let service = services.host_http_egress(network);
    let adapter = ScriptRuntimeHttpAdapter::new(Arc::new(service));

    let response = adapter
        .request(ScriptHostHttpRequest {
            scope: scope.clone(),
            capability_id: capability_id.clone(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: caller_supplied_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            timeout_ms: Some(1000),
        })
        .await
        .expect("script adapter should reach host egress using staged policy");

    assert_eq!(response.status, 202);
    assert_eq!(response.body, b"script-ok".to_vec());
    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].policy, staged_policy);
    assert_eq!(requests[0].url, "https://api.example.test/v1/run");
    assert_eq!(requests[0].body, b"hello".to_vec());
    drop(requests);
}

#[tokio::test]
async fn mcp_http_adapter_borrows_real_host_staged_network_policy() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 203,
        headers: vec![],
        body: b"mcp-ok".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 6,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let services = test_obligation_services();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let staged_policy = sample_policy();
    stage_policy_sync(&services, &scope, &capability_id, staged_policy.clone());
    let service = services.host_http_egress(network);
    let adapter = McpRuntimeHttpAdapter::new(Arc::new(service));

    let response = adapter
        .request(McpHostHttpRequest {
            scope: scope.clone(),
            capability_id: capability_id.clone(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: caller_supplied_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            timeout_ms: Some(1000),
        })
        .await
        .expect("MCP adapter should reach host egress using staged policy");

    assert_eq!(response.status, 203);
    assert_eq!(response.body, b"mcp-ok".to_vec());
    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].policy, staged_policy);
    assert_eq!(requests[0].url, "https://api.example.test/v1/run");
    assert_eq!(requests[0].body, b"hello".to_vec());
    drop(requests);
}

#[tokio::test]
async fn mcp_http_client_reuses_real_host_staged_network_policy_for_json_rpc_session() {
    let network = JsonRpcMcpNetwork::new();
    let network_recorder = network.requests.clone();
    let services = test_obligation_services();
    let scope = sample_scope();
    let capability_id = CapabilityId::new("mcp.search").unwrap();
    let staged_policy = sample_policy();
    stage_policy(&services, &scope, &capability_id, staged_policy.clone()).await;
    let service = services.host_http_egress(network);
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(service)),
        StaticMcpHostHttpEgressPlanner::new(McpHostHttpEgressPlan {
            network_policy: caller_supplied_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            timeout_ms: Some(1000),
        }),
    );

    let output = client
        .call_tool(McpClientRequest {
            provider: ExtensionId::new("mcp").unwrap(),
            capability_id: capability_id.clone(),
            scope: scope.clone(),
            transport: "http".to_string(),
            command: None,
            args: vec![],
            url: Some("https://api.example.test/v1/run".to_string()),
            input: json!({"query": "ironclaw"}),
            max_output_bytes: 4096,
        })
        .await
        .expect("one staged policy must cover the whole MCP JSON-RPC exchange");

    assert_eq!(
        output.output,
        json!({"content":[{"type":"text","text":"ok"}],"isError":false})
    );
    let requests = network_recorder.lock().unwrap();
    assert_eq!(
        requests.len(),
        3,
        "initialize, initialized notification, and tools/call should all reach transport"
    );
    assert!(
        requests
            .iter()
            .all(|request| request.policy == staged_policy)
    );
    drop(requests);
}

#[tokio::test]
async fn mcp_http_client_reuses_staged_credential_for_json_rpc_session() {
    let network = JsonRpcMcpNetwork::new();
    let network_recorder = network.requests.clone();
    let services = test_obligation_services();
    let scope = sample_scope();
    let capability_id = CapabilityId::new("mcp.search").unwrap();
    let handle = SecretHandle::new("github-token").unwrap();
    stage_policy(&services, &scope, &capability_id, sample_policy()).await;
    stage_secret(
        &services,
        &scope,
        &capability_id,
        &handle,
        "sk-staged-mcp-secret",
    )
    .await;
    let service = services.host_http_egress(network);
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(service)),
        StaticMcpHostHttpEgressPlanner::new(McpHostHttpEgressPlan {
            network_policy: caller_supplied_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            timeout_ms: Some(1000),
        }),
    );

    let output = client
        .call_tool(McpClientRequest {
            provider: ExtensionId::new("mcp").unwrap(),
            capability_id: capability_id.clone(),
            scope: scope.clone(),
            transport: "http".to_string(),
            command: None,
            args: vec![],
            url: Some("https://api.example.test/v1/run".to_string()),
            input: json!({"query": "ironclaw"}),
            max_output_bytes: 4096,
        })
        .await
        .expect("staged MCP credential should cover the whole JSON-RPC session");

    assert_eq!(
        output.output,
        json!({"content":[{"type":"text","text":"ok"}],"isError":false})
    );
    let requests = network_recorder.lock().unwrap();
    assert_eq!(
        requests.len(),
        3,
        "initialize, initialized notification, and tools/call should all reach transport"
    );
    assert!(
        requests.iter().all(|request| {
            request.headers.iter().any(|(name, value)| {
                name == "authorization" && value == "Bearer sk-staged-mcp-secret"
            })
        }),
        "every MCP session request must receive the staged credential"
    );
    drop(requests);
}

#[tokio::test]
async fn wasm_http_adapter_reuses_staged_credential_for_multiple_requests_in_one_invocation() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 0,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let services = test_obligation_services();
    let scope = sample_scope();
    let capability_id = CapabilityId::new("google-drive.download_file").unwrap();
    let handle = SecretHandle::new("google_runtime_token").unwrap();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    stage_secret_sync(
        &services,
        &scope,
        &capability_id,
        &handle,
        "sk-staged-wasm-secret",
    );
    let service: Arc<dyn RuntimeHttpEgress> = Arc::new(services.host_http_egress(network));
    let adapter = WasmRuntimeHttpAdapter::new(
        service,
        scope,
        capability_id.clone(),
        caller_supplied_policy(),
    )
    .with_credential_provider(Arc::new(WasmStagedRuntimeCredentials::new(vec![
        WasmStagedRuntimeCredential::for_any_request(
            handle,
            RuntimeCredentialTarget::Header {
                name: "authorization".to_string(),
                prefix: Some("Bearer ".to_string()),
            },
            true,
        ),
    ])));

    for url in [
        "https://api.example.test/drive/v3/files/doc-id?fields=id,name,mimeType",
        "https://api.example.test/drive/v3/files/doc-id/export?mimeType=text%2Fplain",
    ] {
        adapter
            .request(WasmHttpRequest {
                method: "GET".to_string(),
                url: url.to_string(),
                headers_json: "{}".to_string(),
                body: None,
                timeout_ms: None,
            })
            .expect("each Drive request should receive the staged WASM credential");
    }

    let requests = network_recorder.lock().unwrap();
    assert_eq!(
        requests.len(),
        2,
        "Drive download_file should make metadata and export requests"
    );
    assert!(
        requests
            .iter()
            .all(|request| request
                .headers
                .iter()
                .any(|(name, value)| name == "authorization"
                    && value == "Bearer sk-staged-wasm-secret")),
        "each WASM HTTP request in the same invocation must receive the staged credential"
    );
}

#[tokio::test]
async fn mcp_http_client_reuses_product_auth_staged_credential_for_json_rpc_session() {
    let network = JsonRpcMcpNetwork::new();
    let network_recorder = network.requests.clone();
    let source_scope = sample_scope();
    let mut runtime_scope = source_scope.clone();
    runtime_scope.project_id = Some(ProjectId::new("runtime-project").unwrap());
    runtime_scope.invocation_id = InvocationId::new();
    let capability_id = CapabilityId::new("mcp.search").unwrap();
    let account_access_handle = SecretHandle::new("mcp_account_access").unwrap();
    let runtime_slot_handle = SecretHandle::new("mcp_runtime_token").unwrap();
    let services = test_obligation_services().with_credential_account_resolver(Arc::new(
        SourceScopedCredentialAccountResolver {
            source_scope: source_scope.clone(),
            handle: account_access_handle.clone(),
        },
    ));
    services
        .secret_store()
        .put(
            source_scope,
            account_access_handle,
            SecretMaterial::from("sk-account-scope-mcp-secret"),
            None,
        )
        .await
        .unwrap();
    let context = context_for_scope(runtime_scope.clone());
    services
        .obligation_handler()
        .satisfy(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &ResourceEstimate::default(),
            obligations: &[
                Obligation::ApplyNetworkPolicy {
                    policy: sample_policy(),
                },
                Obligation::InjectCredentialAccountOnce {
                    handle: runtime_slot_handle.clone(),
                    provider: RuntimeCredentialAccountProviderId::new("mcp").unwrap(),
                    setup: ironclaw_host_api::RuntimeCredentialAccountSetup::ManualToken,
                    provider_scopes: Vec::new(),
                    requester_extension: ExtensionId::new("mcp").unwrap(),
                },
            ],
        })
        .await
        .expect("credential account obligation should stage from resolved source scope");
    let service = services.host_http_egress(network);
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(service)),
        StaticMcpHostHttpEgressPlanner::new(McpHostHttpEgressPlan {
            network_policy: caller_supplied_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle: runtime_slot_handle,
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            timeout_ms: Some(1000),
        }),
    );

    let output = client
        .call_tool(McpClientRequest {
            provider: ExtensionId::new("mcp").unwrap(),
            capability_id: capability_id.clone(),
            scope: runtime_scope,
            transport: "http".to_string(),
            command: None,
            args: vec![],
            url: Some("https://api.example.test/v1/run".to_string()),
            input: json!({"query": "ironclaw"}),
            max_output_bytes: 4096,
        })
        .await
        .expect("product-auth staged credential should cover the whole MCP JSON-RPC session");

    assert_eq!(
        output.output,
        json!({"content":[{"type":"text","text":"ok"}],"isError":false})
    );
    let requests = network_recorder.lock().unwrap();
    assert_eq!(
        requests.len(),
        3,
        "initialize, initialized notification, and tools/call should all reach transport"
    );
    assert!(
        requests.iter().all(|request| request
            .headers
            .iter()
            .any(|(name, value)| name == "authorization"
                && value == "Bearer sk-account-scope-mcp-secret")),
        "every MCP session request must receive the staged product-auth credential"
    );
    drop(requests);
}

#[tokio::test]
async fn mcp_http_client_cannot_use_direct_secret_store_lease_with_production_egress() {
    let network = JsonRpcMcpNetwork::new();
    let network_recorder = network.requests.clone();
    let services = test_obligation_services();
    let scope = sample_scope();
    let capability_id = CapabilityId::new("mcp.search").unwrap();
    let handle = SecretHandle::new("github-token").unwrap();
    stage_policy(&services, &scope, &capability_id, sample_policy()).await;
    services
        .secret_store()
        .put(
            scope.clone(),
            handle.clone(),
            SecretMaterial::from("sk-direct-lease"),
            None,
        )
        .await
        .unwrap();
    let service = services.host_http_egress(network);
    let client = McpHostHttpClient::new(
        McpRuntimeHttpAdapter::new(Arc::new(service)),
        StaticMcpHostHttpEgressPlanner::new(McpHostHttpEgressPlan {
            network_policy: caller_supplied_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                source: RuntimeCredentialSource::SecretStoreLease,
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            timeout_ms: Some(1000),
        }),
    );

    let error = client
        .call_tool(McpClientRequest {
            provider: ExtensionId::new("mcp").unwrap(),
            capability_id,
            scope,
            transport: "http".to_string(),
            command: None,
            args: vec![],
            url: Some("https://api.example.test/v1/run".to_string()),
            input: json!({"query": "ironclaw"}),
            max_output_bytes: 4096,
        })
        .await
        .expect_err("production MCP egress must require staged credentials");

    assert_eq!(error.stable_reason(), "request_denied");
    let requests = network_recorder.lock().unwrap();
    assert_eq!(
        requests.len(),
        0,
        "MCP direct leases must fail before initialize, initialized, or tools/call transport"
    );
    assert!(requests.iter().all(|request| {
        !request
            .headers
            .iter()
            .any(|(name, _)| name == "authorization")
    }));
}

#[tokio::test]
async fn first_party_http_egress_cannot_use_direct_secret_store_lease_with_production_egress() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let services = test_obligation_services();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    let service = services.host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope,
            capability_id,
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle: SecretHandle::new("api-token").unwrap(),
                source: RuntimeCredentialSource::SecretStoreLease,
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("production first-party egress must require staged credentials");

    assert!(matches!(
        error,
        RuntimeHttpEgressError::Credential { reason }
            if reason == "direct secret-store leases are unavailable for production runtime egress"
    ));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_fails_closed_without_staged_network_policy() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{\"ok\":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let services = test_obligation_services();
    let service = services.host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Wasm,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("missing staged network policy should fail before transport");

    assert!(matches!(
        error,
        RuntimeHttpEgressError::Network {
            reason,
            request_bytes: 0,
            response_bytes: 0,
        } if reason == "network_policy_missing"
    ));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_does_not_use_cross_scope_or_cross_capability_policy() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{\"ok\":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let services = test_obligation_services();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let mut other_scope = scope.clone();
    other_scope.agent_id = Some(AgentId::new("other-agent").unwrap());
    let other_capability_id = CapabilityId::new("other.http").unwrap();
    stage_policy_sync(&services, &other_scope, &capability_id, sample_policy());
    stage_policy_sync(&services, &scope, &other_capability_id, sample_policy());
    let service = services.host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Wasm,
            scope: scope.clone(),
            capability_id: capability_id.clone(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("cross-scope or cross-capability staged policies must not authorize egress");

    assert!(matches!(
        error,
        RuntimeHttpEgressError::Network {
            reason,
            request_bytes: 0,
            response_bytes: 0,
        } if reason == "network_policy_missing"
    ));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_consumes_staged_policy_when_dispatch_fails_before_transport() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{\"ok\":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let services = test_obligation_services();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    let service = services.host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Wasm,
            scope: scope.clone(),
            capability_id: capability_id.clone(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle: SecretHandle::new("missing-token").unwrap(),
                source: RuntimeCredentialSource::SecretStoreLease,
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("credential failure should not leave reusable network policy state");

    assert!(matches!(error, RuntimeHttpEgressError::Credential { .. }));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_consumes_staged_policy_when_request_validation_fails() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{\"ok\":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let services = test_obligation_services();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    let service = services.host_http_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Wasm,
            scope: scope.clone(),
            capability_id: capability_id.clone(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![(
                "Authorization".to_string(),
                "Bearer caller-token".to_string(),
            )],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("request validation failure should not leave reusable policy state");

    assert!(matches!(error, RuntimeHttpEgressError::Request { .. }));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_redacts_injected_credentials_from_runtime_visible_response() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![
            (
                "set-cookie".to_string(),
                "session=sk-test-secret".to_string(),
            ),
            ("x-echo".to_string(), "sk-test-secret".to_string()),
        ],
        body: b"upstream echoed sk-test-secret".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 29,
            resolved_ip: None,
        },
    });
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    stage_secret_sync(&services, &scope, &capability_id, &handle, "sk-test-secret");
    let service = services.host_http_egress(network);

    let response = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope,
            capability_id: capability_id.clone(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect("sanitized response should be returned");

    assert!(response.redaction_applied);
    assert_eq!(
        response.headers,
        vec![("x-echo".to_string(), "[REDACTED]".to_string())]
    );
    assert_eq!(response.body, b"upstream echoed [REDACTED]".to_vec());
}

#[tokio::test]
async fn host_http_egress_redacts_lowercase_percent_encoded_secret_echoes() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![(
            "x-echo".to_string(),
            "secret+with%2fslash%2bplus%3f".to_string(),
        )],
        body: b"upstream echoed secret+with%2fslash%2bplus%3f".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 45,
            resolved_ip: None,
        },
    });
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    stage_secret_sync(
        &services,
        &scope,
        &capability_id,
        &handle,
        "secret with/slash+plus?",
    );
    let service = services.host_http_egress(network);

    let response = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope,
            capability_id: capability_id.clone(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::QueryParam {
                    name: "token".to_string(),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect("lowercase percent-encoded echoed credentials should be redacted");

    assert!(response.redaction_applied);
    assert_eq!(
        response.headers,
        vec![("x-echo".to_string(), "[REDACTED]".to_string())]
    );
    assert_eq!(response.body, b"upstream echoed [REDACTED]".to_vec());
}

#[tokio::test]
async fn host_http_egress_saves_sanitized_response_body_to_store() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![("content-type".to_string(), "text/plain".to_string())],
        body: b"large patch body".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 16,
            resolved_ip: None,
        },
    });
    let store = Arc::new(RecordingBodyStore::default());
    let service = request_policy_staging_egress_with_body_store(network, store.clone());

    let target = save_target("/workspace/pr.diff");
    let response = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: Some(target.clone()),
            timeout_ms: None,
        })
        .await
        .expect("response body should be saved");

    assert_eq!(response.body, Vec::<u8>::new());
    assert_eq!(response.body.capacity(), 0);
    assert_eq!(
        response.saved_body.as_ref().map(|saved| &saved.path),
        Some(&target.path)
    );
    assert_eq!(
        response
            .saved_body
            .as_ref()
            .map(|saved| saved.bytes_written),
        Some(16)
    );
    let writes = store.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].target, target);
    assert_eq!(writes[0].body, b"large patch body".to_vec());
}

#[tokio::test]
async fn host_http_egress_saves_response_body_to_scoped_filesystem_store() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![("content-type".to_string(), "text/plain".to_string())],
        body: b"filesystem patch body".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 21,
            resolved_ip: None,
        },
    });
    let root = Arc::new(InMemoryBackend::new());
    let scoped_filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(&root),
        MountView::new(Vec::new()).unwrap(),
    ));
    let save_mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/workspace").unwrap(),
        MountPermissions::read_write(),
    )])
    .unwrap();
    let service = request_policy_staging_egress_with_body_store(network, scoped_filesystem.clone());

    let target = save_target_with_mount("/workspace/pr.diff", &save_mounts);
    let response = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: Some(target.clone()),
            timeout_ms: None,
        })
        .await
        .expect("response body should be saved through the scoped filesystem");

    assert_eq!(response.body, Vec::<u8>::new());
    assert_eq!(
        response
            .saved_body
            .as_ref()
            .map(|saved| saved.bytes_written),
        Some(21)
    );
    let saved =
        block_on_test(root.read_file(&VirtualPath::new("/projects/workspace/pr.diff").unwrap()))
            .unwrap();
    assert_eq!(saved, b"filesystem patch body".to_vec());
}

#[tokio::test]
async fn host_http_egress_saves_response_body_to_scoped_filesystem_store_from_tokio_task() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![("content-type".to_string(), "text/plain".to_string())],
        body: b"tokio filesystem body".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 21,
            resolved_ip: None,
        },
    });
    let root = Arc::new(InMemoryBackend::new());
    let scoped_filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(&root),
        MountView::new(Vec::new()).unwrap(),
    ));
    let save_mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/workspace").unwrap(),
        MountPermissions::read_write(),
    )])
    .unwrap();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let services = test_obligation_services();
    stage_policy(&services, &scope, &capability_id, sample_policy()).await;
    let service = services.host_http_egress_with_body_store(network, scoped_filesystem.clone());

    let target = save_target_with_mount("/workspace/from-tokio.txt", &save_mounts);
    let response = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope,
            capability_id,
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: Some(target),
            timeout_ms: None,
        })
        .await
        .expect("response body should be saved without nested Tokio runtime panic");

    assert_eq!(
        response
            .saved_body
            .as_ref()
            .map(|saved| saved.bytes_written),
        Some(21)
    );
    let saved = root
        .read_file(&VirtualPath::new("/projects/workspace/from-tokio.txt").unwrap())
        .await
        .unwrap();
    assert_eq!(saved, b"tokio filesystem body".to_vec());
}

#[tokio::test]
async fn host_http_egress_rejects_save_when_target_mount_view_is_read_only() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![("content-type".to_string(), "text/plain".to_string())],
        body: b"filesystem patch body".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 21,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let scoped_filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/projects/workspace").unwrap(),
            MountPermissions::read_write(),
        )])
        .unwrap(),
    ));
    let service = request_policy_staging_egress_with_body_store(network, scoped_filesystem.clone());

    let read_only_mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/workspace").unwrap(),
        MountPermissions::read_only(),
    )])
    .unwrap();
    let target = save_target_with_mount("/workspace/pr.diff", &read_only_mounts);
    let direct_error = scoped_filesystem
        .authorize_write(&sample_scope(), &sample_capability_id(), &target)
        .expect_err("direct body-store authorization should fail");
    assert!(!direct_error.to_string().contains("/workspace/pr.diff"));

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: Some(target.clone()),
            timeout_ms: None,
        })
        .await
        .expect_err("read-only target mount should deny saving before network");

    assert_eq!(error.stable_runtime_reason(), "request_denied");
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_save_target_requires_configured_body_store_before_network() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: b"large patch body".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 16,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let service = request_policy_staging_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: Some(save_target("/workspace/pr.diff")),
            timeout_ms: None,
        })
        .await
        .expect_err("missing body store should fail closed");

    assert!(matches!(error, RuntimeHttpEgressError::Request { .. }));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_save_target_requires_write_authorization_before_network() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: b"large patch body".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 16,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let store = Arc::new(
        RecordingBodyStore::default()
            .with_authorize_error("write permission denied for /workspace/pr.diff"),
    );
    let service = request_policy_staging_egress_with_body_store(network, store);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: Some(save_target("/workspace/pr.diff")),
            timeout_ms: None,
        })
        .await
        .expect_err("unauthorized save target should fail closed");

    match error {
        RuntimeHttpEgressError::Request {
            reason,
            request_bytes,
            response_bytes,
        } => {
            assert_eq!(request_bytes, 0);
            assert_eq!(response_bytes, 0);
            assert_eq!(reason, "response_body_store_unauthorized");
        }
        other => panic!("expected request authorization error, got {other:?}"),
    }
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_rejects_save_when_body_store_is_unavailable() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: b"large patch body".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 16,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let store = Arc::new(RecordingBodyStore::default().with_authorize_unavailable());
    let service = request_policy_staging_egress_with_body_store(network, store);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: Some(save_target("/workspace/pr.diff")),
            timeout_ms: None,
        })
        .await
        .expect_err("unavailable body store should fail closed");

    match error {
        RuntimeHttpEgressError::Request {
            reason,
            request_bytes,
            response_bytes,
        } => {
            assert_eq!(request_bytes, 0);
            assert_eq!(response_bytes, 0);
            assert_eq!(reason, "response_body_store_unavailable");
        }
        other => panic!("expected body store unavailable error, got {other:?}"),
    }
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_discards_staged_secret_on_pre_injection_error() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: b"large patch body".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 16,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let store = Arc::new(RecordingBodyStore::default().with_authorize_unavailable());
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    stage_secret_sync(
        &services,
        &scope,
        &capability_id,
        &handle,
        "sk-staged-secret",
    );
    let service = services.host_http_egress_with_body_store(network, store);

    service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope: scope.clone(),
            capability_id: capability_id.clone(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle: handle.clone(),
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: Some(save_target("/workspace/pr.diff")),
            timeout_ms: None,
        })
        .await
        .expect_err("pre-injection body-store failure should fail closed");

    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope: scope.clone(),
            capability_id: capability_id.clone(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle: handle.clone(),
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("pre-injection failure should discard staged secrets");

    assert!(matches!(
        error,
        RuntimeHttpEgressError::Credential { ref reason }
            if reason == "required credential is unavailable"
    ));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[test]
fn tool_call_http_egress_discards_staged_policy_and_secret_on_pre_transport_error() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: b"large patch body".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 16,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let store = Arc::new(RecordingBodyStore::default().with_authorize_unavailable());
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    stage_secret_sync(
        &services,
        &scope,
        &capability_id,
        &handle,
        "sk-staged-secret",
    );
    let service = services.host_http_egress_with_body_store(network, store);

    block_on_test(
        service.execute_for_model_visible_output(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope: scope.clone(),
            capability_id: capability_id.clone(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle: handle.clone(),
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: Some(save_target("/workspace/pr.diff")),
            timeout_ms: None,
        }),
    )
    .expect_err("tool-call pre-transport failure should fail closed");

    let policy_retry = block_on_test(service.execute_for_model_visible_output(
        RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope: scope.clone(),
            capability_id: capability_id.clone(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        },
    ))
    .expect_err("pre-transport failure should discard staged policy");

    assert!(matches!(
        policy_retry,
        RuntimeHttpEgressError::Network {
            ref reason,
            request_bytes: 0,
            response_bytes: 0,
        } if reason == "network_policy_missing"
    ));

    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    let secret_retry = block_on_test(service.execute_for_model_visible_output(
        RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope: scope.clone(),
            capability_id: capability_id.clone(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        },
    ))
    .expect_err("pre-transport failure should discard staged secret");

    assert!(matches!(
        secret_retry,
        RuntimeHttpEgressError::Credential { ref reason }
            if reason == "required credential is unavailable"
    ));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_fails_closed_when_body_store_write_fails() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![("content-type".to_string(), "text/plain".to_string())],
        body: b"large patch body".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 16,
            resolved_ip: None,
        },
    });
    let store = Arc::new(RecordingBodyStore::default().with_write_error("disk full"));
    let service = request_policy_staging_egress_with_body_store(network, store.clone());

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: Some(save_target("/workspace/pr.diff")),
            timeout_ms: None,
        })
        .await
        .expect_err("body store write failure should fail closed");

    match error {
        RuntimeHttpEgressError::Response {
            reason,
            request_bytes,
            response_bytes,
        } => {
            assert_eq!(request_bytes, 5);
            assert_eq!(response_bytes, 16);
            assert_eq!(reason, "response_body_store_failed");
        }
        other => panic!("expected response body store error, got {other:?}"),
    }
    assert!(store.writes().is_empty());
}

#[tokio::test]
async fn host_http_egress_fails_closed_when_real_scoped_filesystem_write_fails() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![("content-type".to_string(), "text/plain".to_string())],
        body: b"large patch body".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 16,
            resolved_ip: None,
        },
    });
    let temp = tempdir().unwrap();
    let mut root = LocalFilesystem::new();
    root.mount_local(
        VirtualPath::new("/projects/workspace").unwrap(),
        ironclaw_host_api::HostPath::from_path_buf(temp.path().to_path_buf()),
    )
    .unwrap();
    let scoped_filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(root),
        MountView::new(Vec::new()).unwrap(),
    ));
    fs::write(temp.path().join("dir"), b"blocking file").unwrap();
    let service = request_policy_staging_egress_with_body_store(network, scoped_filesystem);
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/workspace").unwrap(),
        MountPermissions::read_write(),
    )])
    .unwrap();
    let target = save_target_with_mount("/workspace/dir/file.txt", &mounts);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: Some(target),
            timeout_ms: None,
        })
        .await
        .expect_err("real scoped filesystem write failure should fail closed");

    match error {
        RuntimeHttpEgressError::Response {
            reason,
            request_bytes,
            response_bytes,
        } => {
            assert_eq!(request_bytes, 5);
            assert_eq!(response_bytes, 16);
            assert_eq!(reason, "response_body_store_failed");
        }
        other => panic!("expected response body store error, got {other:?}"),
    }
    assert_eq!(
        fs::read(temp.path().join("dir")).unwrap(),
        b"blocking file".to_vec()
    );
}

#[tokio::test]
async fn host_http_egress_saves_redacted_response_body() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: b"upstream echoed sk-test-secret".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 29,
            resolved_ip: None,
        },
    });
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    stage_secret_sync(&services, &scope, &capability_id, &handle, "sk-test-secret");
    let store = Arc::new(RecordingBodyStore::default());
    let service = services.host_http_egress_with_body_store(network, store.clone());

    let response = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope,
            capability_id: capability_id.clone(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            save_body_to: Some(save_target("/workspace/redacted.txt")),
            timeout_ms: None,
        })
        .await
        .expect("sanitized response body should be saved");

    assert!(response.redaction_applied);
    assert_eq!(response.body, Vec::<u8>::new());
    let writes = store.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].body, b"upstream echoed [REDACTED]".to_vec());
}

#[tokio::test]
async fn host_http_egress_strips_all_sensitive_response_headers() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![
            ("api-key".to_string(), "short-manual-key".to_string()),
            ("x-token".to_string(), "short-manual-key".to_string()),
            ("x-access-token".to_string(), "short-manual-key".to_string()),
            (
                "x-session-token".to_string(),
                "short-manual-key".to_string(),
            ),
            ("x-csrf-token".to_string(), "short-manual-key".to_string()),
            ("x-refresh-token".to_string(), "opaque-refresh".to_string()),
            (
                "x-amz-security-token".to_string(),
                "opaque-session".to_string(),
            ),
            ("private-token".to_string(), "opaque-private".to_string()),
            ("x-credential".to_string(), "opaque-credential".to_string()),
            ("x-secret".to_string(), "short-manual-key".to_string()),
            ("x-api-secret".to_string(), "short-manual-key".to_string()),
            ("x-public".to_string(), "ok".to_string()),
        ],
        body: b"{}".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 2,
            resolved_ip: None,
        },
    });
    let service = request_policy_staging_egress(network);

    let response = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect("sensitive response headers should be stripped before runtime visibility");

    assert!(response.redaction_applied);
    assert_eq!(
        response.headers,
        vec![("x-public".to_string(), "ok".to_string())]
    );
}

#[tokio::test]
async fn host_http_egress_blocks_credential_shaped_response_body() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: b"leaked key sk-proj-test1234567890abcdefghij".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 43,
            resolved_ip: None,
        },
    });
    let service = request_policy_staging_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("credential-shaped response bodies should not reach runtimes");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Response { ref reason, .. }
            if reason == "response_leak_blocked"
    ));
    assert!(!error.to_string().contains("sk-proj-test"));
    assert_eq!(error.request_bytes(), 5);
    assert_eq!(error.response_bytes(), 43);
}

#[tokio::test]
async fn host_http_egress_blocks_percent_encoded_credential_path_before_network() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let service = request_policy_staging_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/%73%6b%2d%70%72%6f%6a%2dtest1234567890abcdefghij/run"
                .to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("percent-encoded credential-shaped path should fail before network dispatch");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Request { ref reason, .. }
            if reason == "credential_leak_blocked"
    ));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_blocks_credential_shaped_runtime_request_before_network() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let service = request_policy_staging_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"sk-proj-test1234567890abcdefghij".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("credential-shaped runtime requests should fail before network dispatch");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Request { ref reason, .. }
            if reason == "credential_leak_blocked"
    ));
    assert!(!error.to_string().contains("sk-proj-test"));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_blocks_runtime_supplied_sensitive_headers_before_network() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let service = request_policy_staging_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![(
                "Authorization".to_string(),
                "Bearer caller-token".to_string(),
            )],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("runtime-supplied sensitive headers should fail before network dispatch");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Request { .. }
    ));
    assert!(error.to_string().contains("sensitive_header"));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_blocks_leaky_response_header_values() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![(
            "x-note".to_string(),
            "leaked sk-proj-test1234567890abcdefghij".to_string(),
        )],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let service = request_policy_staging_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("leaky response headers should fail closed");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Response { ref reason, .. }
            if reason == "response_leak_blocked"
    ));
}

#[tokio::test]
async fn host_http_egress_blocks_runtime_supplied_credential_query_before_network() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let service = request_policy_staging_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/run?api_key=short-manual-key".to_string(),
            headers: vec![],
            body: Vec::new(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("runtime-supplied credential query params should fail before network dispatch");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Request { .. }
    ));
    assert!(error.to_string().contains("manual_credentials"));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_blocks_percent_encoded_credential_values_before_network() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let service = request_policy_staging_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/run?data=AKIA%49OSFODNN7EXAMPLE".to_string(),
            headers: vec![],
            body: Vec::new(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("percent-encoded credential values should fail before network dispatch");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Request { .. }
    ));
    assert!(!error.to_string().contains("AKIAIOSFODNN7EXAMPLE"));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_blocks_runtime_supplied_auth_like_headers_before_network() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let service = request_policy_staging_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![("X-Custom-Auth".to_string(), "short-manual-key".to_string())],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("runtime-supplied auth-like headers should fail before network dispatch");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Request { .. }
    ));
    assert!(error.to_string().contains("manual_credentials"));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_http_egress_maps_network_errors_to_stable_runtime_reasons() {
    let network = RecordingNetwork::err(NetworkHttpError::Transport {
        reason: "connection failed for https://api.example.test/path?token=raw-secret".to_string(),
        request_bytes: 12,
        response_bytes: 0,
    });
    let service = request_policy_staging_egress(network);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        })
        .await
        .expect_err("network errors should surface as stable sanitized variants");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Network { .. }
    ));
    assert!(error.to_string().contains("transport_failed"));
    assert!(!error.to_string().contains("raw-secret"));
    assert!(!error.to_string().contains("api.example.test/path"));
    assert_eq!(error.request_bytes(), 12);
}

#[derive(Clone)]
struct RecordingNetwork {
    response: Result<NetworkHttpResponse, NetworkHttpError>,
    requests: Arc<Mutex<Vec<NetworkHttpRequest>>>,
}

#[derive(Debug, Clone, Default)]
struct RecordingBodyStore {
    writes: Arc<Mutex<Vec<RecordedBodyWrite>>>,
    authorize_error: Arc<Mutex<Option<RuntimeHttpBodyStoreError>>>,
    write_error: Arc<Mutex<Option<RuntimeHttpBodyStoreError>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedBodyWrite {
    scope: ResourceScope,
    capability_id: CapabilityId,
    target: RuntimeHttpSaveTarget,
    body: Vec<u8>,
}

impl RecordingBodyStore {
    fn with_authorize_error(self, reason: &str) -> Self {
        *self.authorize_error.lock().unwrap() = Some(RuntimeHttpBodyStoreError::Unauthorized {
            reason: reason.to_string(),
        });
        self
    }

    fn with_authorize_unavailable(self) -> Self {
        *self.authorize_error.lock().unwrap() = Some(RuntimeHttpBodyStoreError::Unavailable);
        self
    }

    fn with_write_error(self, reason: &str) -> Self {
        *self.write_error.lock().unwrap() = Some(RuntimeHttpBodyStoreError::Failed {
            reason: reason.to_string(),
        });
        self
    }

    fn writes(&self) -> Vec<RecordedBodyWrite> {
        self.writes.lock().unwrap().clone()
    }
}

impl RuntimeHttpBodyStore for RecordingBodyStore {
    fn authorize_write(
        &self,
        _scope: &ResourceScope,
        _capability_id: &CapabilityId,
        _target: &RuntimeHttpSaveTarget,
    ) -> Result<(), RuntimeHttpBodyStoreError> {
        if let Some(error) = self.authorize_error.lock().unwrap().clone() {
            return Err(error);
        }
        Ok(())
    }

    fn write_body(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        target: &RuntimeHttpSaveTarget,
        body: &[u8],
    ) -> Result<(), RuntimeHttpBodyStoreError> {
        if let Some(error) = self.write_error.lock().unwrap().clone() {
            return Err(error);
        }
        self.writes.lock().unwrap().push(RecordedBodyWrite {
            scope: scope.clone(),
            capability_id: capability_id.clone(),
            target: target.clone(),
            body: body.to_vec(),
        });
        Ok(())
    }
}

#[derive(Clone)]
struct JsonRpcMcpNetwork {
    requests: Arc<Mutex<Vec<NetworkHttpRequest>>>,
}

#[derive(Clone)]
struct UrlEchoNetwork {
    requests: Arc<Mutex<Vec<NetworkHttpRequest>>>,
}

impl UrlEchoNetwork {
    fn new() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait::async_trait]
impl NetworkHttpEgress for UrlEchoNetwork {
    async fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        self.requests.lock().unwrap().push(request.clone());
        Err(NetworkHttpError::Transport {
            reason: format!("upstream rejected {}", request.url),
            request_bytes: request.body.len() as u64,
            response_bytes: 0,
        })
    }
}

impl JsonRpcMcpNetwork {
    fn new() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait::async_trait]
impl NetworkHttpEgress for JsonRpcMcpNetwork {
    async fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let request_bytes = request.body.len() as u64;
        let body = serde_json::from_slice::<Value>(&request.body).map_err(|error| {
            NetworkHttpError::Transport {
                reason: format!("invalid JSON-RPC request: {error}"),
                request_bytes,
                response_bytes: 0,
            }
        })?;
        let method = body
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let id = body.get("id").cloned().unwrap_or(Value::Null);
        self.requests.lock().unwrap().push(request);

        let (status, headers, response_body) = match method.as_str() {
            "initialize" => (
                200,
                vec![("Mcp-Session-Id".to_string(), "session-123".to_string())],
                json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"test","version":"1"}}}),
            ),
            "notifications/initialized" => (202, Vec::new(), Value::Null),
            "tools/call" => (
                200,
                Vec::new(),
                json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":"ok"}],"isError":false}}),
            ),
            _ => (
                500,
                Vec::new(),
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"method not found"}}),
            ),
        };
        let body = if status == 202 {
            Vec::new()
        } else {
            serde_json::to_vec(&response_body).unwrap()
        };
        Ok(NetworkHttpResponse {
            status,
            headers,
            usage: NetworkUsage {
                request_bytes,
                response_bytes: body.len() as u64,
                resolved_ip: None,
            },
            body,
        })
    }
}

impl RecordingNetwork {
    fn ok(response: NetworkHttpResponse) -> Self {
        Self {
            response: Ok(response),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn err(error: NetworkHttpError) -> Self {
        Self {
            response: Err(error),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait::async_trait]
impl NetworkHttpEgress for RecordingNetwork {
    async fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        self.requests.lock().unwrap().push(request);
        self.response.clone()
    }
}

type RecordedRequests = Arc<Mutex<Vec<NetworkHttpRequest>>>;
type PathPlaceholderEgressResult = Result<
    (RuntimeHttpEgressResponse, RecordedRequests),
    (RuntimeHttpEgressError, RecordedRequests),
>;

async fn execute_path_placeholder_egress(
    url: &str,
    placeholder: &str,
    material: &str,
) -> PathPlaceholderEgressResult {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    let services = test_obligation_services();
    stage_policy_sync(&services, &scope, &capability_id, sample_policy());
    stage_secret_sync(&services, &scope, &capability_id, &handle, material);
    let service = services.host_http_egress(network);

    let response = service.execute(RuntimeHttpEgressRequest {
        runtime: RuntimeKind::Script,
        scope,
        capability_id: capability_id.clone(),
        method: NetworkMethod::Post,
        url: url.to_string(),
        headers: vec![],
        body: b"hello".to_vec(),
        network_policy: sample_policy(),
        credential_injections: vec![RuntimeCredentialInjection {
            handle,
            source: RuntimeCredentialSource::StagedObligation { capability_id },
            target: RuntimeCredentialTarget::PathPlaceholder {
                placeholder: placeholder.to_string(),
            },
            required: true,
        }],
        response_body_limit: Some(4096),
        save_body_to: None,
        timeout_ms: None,
    });

    response
        .await
        .map(|response| (response, network_recorder.clone()))
        .map_err(|error| (error, network_recorder))
}

fn credential_reason(error: &RuntimeHttpEgressError) -> &str {
    match error {
        RuntimeHttpEgressError::Credential { reason } => reason,
        other => panic!("expected credential error, got {other:?}"),
    }
}

fn block_on_test<T, F>(future: F) -> T
where
    T: Send,
    F: std::future::Future<Output = T> + Send,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        return std::thread::scope(|scope| scope.spawn(|| block_on_test_runtime(future)).join())
            .unwrap();
    }

    block_on_test_runtime(future)
}

fn block_on_test_runtime<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn test_obligation_services() -> BuiltinObligationServices {
    BuiltinObligationServices::new(
        Arc::new(InMemoryAuditSink::new()),
        Arc::new(InMemorySecretStore::new()),
        Arc::new(InMemoryResourceGovernor::new()),
    )
}

struct RequestPolicyStagingEgress {
    services: BuiltinObligationServices,
    inner: Arc<dyn RuntimeHttpEgress>,
}

#[derive(Debug)]
struct SourceScopedCredentialAccountResolver {
    source_scope: ResourceScope,
    handle: SecretHandle,
}

#[async_trait::async_trait]
impl RuntimeCredentialAccountResolver for SourceScopedCredentialAccountResolver {
    async fn resolve_access_secret(
        &self,
        _request: RuntimeCredentialAccountRequest<'_>,
    ) -> Result<RuntimeCredentialAccessSecret, CredentialStageError> {
        Ok(RuntimeCredentialAccessSecret {
            scope: self.source_scope.clone(),
            handle: self.handle.clone(),
        })
    }
}

#[async_trait::async_trait]
impl RuntimeHttpEgress for RequestPolicyStagingEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        stage_policy_sync(
            &self.services,
            &request.scope,
            &request.capability_id,
            request.network_policy.clone(),
        );
        self.inner.execute(request).await
    }
}

fn request_policy_staging_egress<N>(network: N) -> Arc<dyn RuntimeHttpEgress>
where
    N: NetworkHttpEgress + 'static,
{
    let services = test_obligation_services();
    let inner = Arc::new(services.host_http_egress(network));
    Arc::new(RequestPolicyStagingEgress { services, inner })
}

fn request_policy_staging_egress_with_body_store<N, T>(
    network: N,
    body_store: Arc<T>,
) -> Arc<dyn RuntimeHttpEgress>
where
    N: NetworkHttpEgress + 'static,
    T: RuntimeHttpBodyStore + 'static,
{
    let services = test_obligation_services();
    let inner = Arc::new(services.host_http_egress_with_body_store(network, body_store));
    Arc::new(RequestPolicyStagingEgress { services, inner })
}

fn context_for_scope(scope: ResourceScope) -> ExecutionContext {
    let mut context = execution_context();
    context.resource_scope = scope;
    context
}

fn stage_policy_sync(
    services: &BuiltinObligationServices,
    scope: &ResourceScope,
    capability_id: &CapabilityId,
    policy: NetworkPolicy,
) {
    block_on_test(stage_policy(services, scope, capability_id, policy));
}

async fn stage_policy(
    services: &BuiltinObligationServices,
    scope: &ResourceScope,
    capability_id: &CapabilityId,
    policy: NetworkPolicy,
) {
    let context = context_for_scope(scope.clone());
    services
        .obligation_handler()
        .satisfy(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id,
            estimate: &ResourceEstimate::default(),
            obligations: &[Obligation::ApplyNetworkPolicy { policy }],
        })
        .await
        .unwrap();
}

fn stage_secret_sync(
    services: &BuiltinObligationServices,
    scope: &ResourceScope,
    capability_id: &CapabilityId,
    handle: &SecretHandle,
    material: &str,
) {
    block_on_test(stage_secret(
        services,
        scope,
        capability_id,
        handle,
        material,
    ));
}

async fn stage_secret(
    services: &BuiltinObligationServices,
    scope: &ResourceScope,
    capability_id: &CapabilityId,
    handle: &SecretHandle,
    material: &str,
) {
    services
        .secret_store()
        .put(
            scope.clone(),
            handle.clone(),
            SecretMaterial::from(material),
            None,
        )
        .await
        .unwrap();
    let context = context_for_scope(scope.clone());
    services
        .obligation_handler()
        .satisfy(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id,
            estimate: &ResourceEstimate::default(),
            obligations: &[Obligation::InjectSecretOnce {
                handle: handle.clone(),
            }],
        })
        .await
        .unwrap();
}

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn execution_context() -> ExecutionContext {
    ExecutionContext::local_default(
        UserId::new("user1").unwrap(),
        ExtensionId::new("example").unwrap(),
        RuntimeKind::Script,
        TrustClass::Sandbox,
        CapabilitySet::default(),
        MountView::default(),
    )
    .unwrap()
}

fn sample_capability_id() -> CapabilityId {
    CapabilityId::new("runtime.http").unwrap()
}

fn save_target(path: &str) -> RuntimeHttpSaveTarget {
    RuntimeHttpSaveTarget {
        path: ScopedPath::new(path).unwrap(),
        mount_grant: None,
    }
}

fn save_target_with_mount(path: &str, mounts: &MountView) -> RuntimeHttpSaveTarget {
    let scoped_path = mounts.scoped_path(path.to_string()).unwrap();
    let (virtual_path, grant) = mounts.resolve_with_grant(&scoped_path).unwrap();
    RuntimeHttpSaveTarget {
        mount_grant: Some(MountGrant::new(
            MountAlias::new(path).unwrap(),
            virtual_path,
            MountPermissions {
                read: false,
                write: grant.permissions.write,
                delete: false,
                list: false,
                execute: false,
            },
        )),
        path: scoped_path,
    }
}

fn sample_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(4096),
    }
}

fn caller_supplied_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "caller.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: false,
        max_egress_bytes: Some(1),
    }
}

fn local_http_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Http),
            host_pattern: "127.0.0.1".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: false,
        max_egress_bytes: Some(1024),
    }
}

fn single_response_server(response: &'static str) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request).unwrap();
        stream.write_all(response.as_bytes()).unwrap();
    });
    (format!("http://127.0.0.1:{port}/test"), handle)
}
