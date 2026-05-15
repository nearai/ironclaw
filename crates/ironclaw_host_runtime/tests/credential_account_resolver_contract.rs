use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_extensions::{HostApiId, ManifestSectionPath};
use ironclaw_host_api::{
    AgentId, CapabilityId, ExtensionId, InvocationId, MissionId, NetworkMethod, ProjectId,
    ResourceScope, RuntimeCredentialTarget, SecretHandle, TenantId, ThreadId, UserId,
};
use ironclaw_host_runtime::{
    CredentialAccountResolver, CredentialAccountResolverError, CredentialAccountResolverRequest,
    CredentialAccountSecretRequirement, HostApiCredentialRequirement,
};
use ironclaw_secrets::{
    CredentialAccount, CredentialAccountId, CredentialAccountStatus, CredentialBrokerError,
    CredentialPathPolicy, CredentialTargetPolicy, InMemoryCredentialBroker, RedactedJson,
};
use ironclaw_wasm::{
    WasmRuntimeCredentialProvider, WasmRuntimeCredentialRequest, WasmStagedRuntimeCredentials,
};
use serde_json::json;

#[tokio::test]
async fn resolves_projected_host_api_requirement_into_exact_url_wasm_credentials() {
    let scope = sample_scope();
    let account_id = CredentialAccountId::new("telegram_bot").unwrap();
    let handle = SecretHandle::new("telegram_bot_token").unwrap();
    let store = Arc::new(InMemoryCredentialBroker::new());
    store
        .put_account(sample_account(&scope, &account_id, &handle))
        .unwrap();
    let request = resolver_request(&scope, "https://api.telegram.org/bot123/sendMessage");

    let resolver = CredentialAccountResolver::new(
        Arc::clone(&store),
        [requirement(&account_id, &handle, None, true)],
    );
    let resolved = resolver.resolve_for_wasm(&request).await.unwrap();

    assert_eq!(
        resolved.secret_requirements,
        vec![CredentialAccountSecretRequirement {
            handle: handle.clone(),
            required: true,
        }]
    );
    assert_eq!(resolved.wasm_credentials.len(), 1);

    let provider = WasmStagedRuntimeCredentials::new(resolved.wasm_credentials);
    let injections = provider.credential_injections(&wasm_request(
        &request,
        "https://api.telegram.org/bot123/sendMessage",
    ));
    assert_eq!(injections.unwrap().len(), 1);
    let injection = provider
        .credential_injections(&wasm_request_with_method(
            &request,
            NetworkMethod::Get,
            "https://api.telegram.org/bot123/sendMessage",
        ))
        .unwrap();
    assert!(
        injection.is_empty(),
        "resolved credential must be exact-method scoped"
    );
    let injection = provider
        .credential_injections(&wasm_request(
            &request,
            "https://api.telegram.org/bot123/deleteMessage",
        ))
        .unwrap();
    assert!(
        injection.is_empty(),
        "resolved credential must be exact-URL scoped"
    );
}

#[tokio::test]
async fn resolves_account_from_same_account_scope_under_different_invocation() {
    let scope = sample_scope();
    let mut stored_scope = scope.clone();
    stored_scope.invocation_id = InvocationId::new();
    let account_id = CredentialAccountId::new("telegram_bot").unwrap();
    let handle = SecretHandle::new("telegram_bot_token").unwrap();
    let store = Arc::new(InMemoryCredentialBroker::new());
    store
        .put_account(sample_account(&stored_scope, &account_id, &handle))
        .unwrap();
    let request = resolver_request(&scope, "https://api.telegram.org/bot123/sendMessage");

    let resolver = CredentialAccountResolver::new(
        Arc::clone(&store),
        [requirement(&account_id, &handle, None, true)],
    );
    let resolved = resolver.resolve_for_wasm(&request).await.unwrap();

    assert_eq!(
        resolved.secret_requirements,
        vec![CredentialAccountSecretRequirement {
            handle,
            required: true,
        }]
    );
    assert_eq!(resolved.wasm_credentials.len(), 1);
}

#[tokio::test]
async fn required_missing_account_fails_closed() {
    let scope = sample_scope();
    let account_id = CredentialAccountId::new("telegram_bot").unwrap();
    let handle = SecretHandle::new("telegram_bot_token").unwrap();
    let resolver = CredentialAccountResolver::new(
        Arc::new(InMemoryCredentialBroker::new()),
        [requirement(&account_id, &handle, None, true)],
    );

    let error = resolver
        .resolve_for_wasm(&resolver_request(
            &scope,
            "https://api.telegram.org/bot123/sendMessage",
        ))
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        CredentialAccountResolverError::Broker(CredentialBrokerError::MissingCredential { .. })
    ));
}

#[tokio::test]
async fn optional_missing_account_is_skipped() {
    let scope = sample_scope();
    let account_id = CredentialAccountId::new("telegram_bot").unwrap();
    let handle = SecretHandle::new("telegram_bot_token").unwrap();
    let resolver = CredentialAccountResolver::new(
        Arc::new(InMemoryCredentialBroker::new()),
        [requirement(&account_id, &handle, None, false)],
    );

    let resolved = resolver
        .resolve_for_wasm(&resolver_request(
            &scope,
            "https://api.telegram.org/bot123/sendMessage",
        ))
        .await
        .unwrap();

    assert!(resolved.secret_requirements.is_empty());
    assert!(resolved.wasm_credentials.is_empty());
}

#[tokio::test]
async fn optional_existing_account_preserves_optional_secret_requirement() {
    let scope = sample_scope();
    let account_id = CredentialAccountId::new("telegram_bot").unwrap();
    let handle = SecretHandle::new("telegram_bot_token").unwrap();
    let store = Arc::new(InMemoryCredentialBroker::new());
    store
        .put_account(sample_account(&scope, &account_id, &handle))
        .unwrap();
    let resolver = CredentialAccountResolver::new(
        Arc::clone(&store),
        [requirement(&account_id, &handle, None, false)],
    );

    let resolved = resolver
        .resolve_for_wasm(&resolver_request(
            &scope,
            "https://api.telegram.org/bot123/sendMessage",
        ))
        .await
        .unwrap();

    assert_eq!(
        resolved.secret_requirements,
        vec![CredentialAccountSecretRequirement {
            handle,
            required: false,
        }]
    );
    assert_eq!(resolved.wasm_credentials.len(), 1);
    assert!(!resolved.wasm_credentials[0].required);
}

#[tokio::test]
async fn duplicate_secret_requirement_keeps_required_semantics() {
    let scope = sample_scope();
    let account_id = CredentialAccountId::new("telegram_bot").unwrap();
    let handle = SecretHandle::new("telegram_bot_token").unwrap();
    let store = Arc::new(InMemoryCredentialBroker::new());
    store
        .put_account(sample_account(&scope, &account_id, &handle))
        .unwrap();
    let resolver = CredentialAccountResolver::new(
        Arc::clone(&store),
        [
            requirement(&account_id, &handle, None, false),
            requirement(&account_id, &handle, None, true),
        ],
    );

    let resolved = resolver
        .resolve_for_wasm(&resolver_request(
            &scope,
            "https://api.telegram.org/bot123/sendMessage",
        ))
        .await
        .unwrap();

    assert_eq!(
        resolved.secret_requirements,
        vec![CredentialAccountSecretRequirement {
            handle,
            required: true,
        }]
    );
    assert_eq!(resolved.wasm_credentials.len(), 1);
    assert!(resolved.wasm_credentials[0].required);
}

#[tokio::test]
async fn rejects_account_from_different_extension() {
    let scope = sample_scope();
    let account_id = CredentialAccountId::new("telegram_bot").unwrap();
    let handle = SecretHandle::new("telegram_bot_token").unwrap();
    let mut account = sample_account(&scope, &account_id, &handle);
    account.provider_or_extension_id = ExtensionId::new("slack").unwrap();
    let store = Arc::new(InMemoryCredentialBroker::new());
    store.put_account(account).unwrap();
    let resolver = CredentialAccountResolver::new(
        Arc::clone(&store),
        [requirement(&account_id, &handle, None, true)],
    );

    let error = resolver
        .resolve_for_wasm(&resolver_request(
            &scope,
            "https://api.telegram.org/bot123/sendMessage",
        ))
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        CredentialAccountResolverError::Broker(
            CredentialBrokerError::CredentialExtensionMismatch { .. }
        )
    ));
}

#[tokio::test]
async fn rejects_store_returning_different_account_id() {
    let scope = sample_scope();
    let requested_account_id = CredentialAccountId::new("telegram_bot").unwrap();
    let returned_account_id = CredentialAccountId::new("other_telegram_bot").unwrap();
    let handle = SecretHandle::new("telegram_bot_token").unwrap();
    let store = Arc::new(MismatchedAccountStore {
        account: sample_account(&scope, &returned_account_id, &handle),
    });
    let resolver = CredentialAccountResolver::new(
        Arc::clone(&store),
        [requirement(&requested_account_id, &handle, None, true)],
    );

    let error = resolver
        .resolve_for_wasm(&resolver_request(
            &scope,
            "https://api.telegram.org/bot123/sendMessage",
        ))
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        CredentialAccountResolverError::Broker(
            CredentialBrokerError::CredentialScopeMismatch { .. }
        )
    ));
}

#[tokio::test]
async fn rejects_request_outside_account_target_policy() {
    let scope = sample_scope();
    let account_id = CredentialAccountId::new("telegram_bot").unwrap();
    let handle = SecretHandle::new("telegram_bot_token").unwrap();
    let store = Arc::new(InMemoryCredentialBroker::new());
    store
        .put_account(sample_account(&scope, &account_id, &handle))
        .unwrap();
    let resolver = CredentialAccountResolver::new(
        Arc::clone(&store),
        [requirement(&account_id, &handle, None, true)],
    );

    let error = resolver
        .resolve_for_wasm(&resolver_request(
            &scope,
            "https://evil.example.com/bot123/sendMessage",
        ))
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        CredentialAccountResolverError::Broker(
            CredentialBrokerError::CredentialPolicyMismatch { .. }
        )
    ));
}

#[tokio::test]
async fn rejects_requirement_handle_not_bound_to_account() {
    let scope = sample_scope();
    let account_id = CredentialAccountId::new("telegram_bot").unwrap();
    let account_handle = SecretHandle::new("telegram_bot_token").unwrap();
    let requirement_handle = SecretHandle::new("telegram_admin_token").unwrap();
    let store = Arc::new(InMemoryCredentialBroker::new());
    store
        .put_account(sample_account(&scope, &account_id, &account_handle))
        .unwrap();
    let resolver = CredentialAccountResolver::new(
        Arc::clone(&store),
        [requirement(&account_id, &requirement_handle, None, true)],
    );

    let error = resolver
        .resolve_for_wasm(&resolver_request(
            &scope,
            "https://api.telegram.org/bot123/sendMessage",
        ))
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        CredentialAccountResolverError::MissingSecretHandle { .. }
    ));
}

fn requirement(
    account_id: &CredentialAccountId,
    handle: &SecretHandle,
    exact_url: Option<&str>,
    required: bool,
) -> HostApiCredentialRequirement {
    HostApiCredentialRequirement {
        extension_id: extension_id(),
        host_api_id: HostApiId::new("ironclaw.capability_provider/v1").unwrap(),
        section_path: ManifestSectionPath::new("capability_provider.tools").unwrap(),
        capability_id: capability_id(),
        account_id: account_id.clone(),
        handle: handle.clone(),
        target: RuntimeCredentialTarget::Header {
            name: "authorization".to_string(),
            prefix: Some("Bearer ".to_string()),
        },
        required,
        exact_url: exact_url.map(str::to_string),
    }
}

fn resolver_request(scope: &ResourceScope, url: &str) -> CredentialAccountResolverRequest {
    CredentialAccountResolverRequest {
        scope: scope.clone(),
        extension_id: extension_id(),
        host_api_id: HostApiId::new("ironclaw.capability_provider/v1").unwrap(),
        section_path: ManifestSectionPath::new("capability_provider.tools").unwrap(),
        capability_id: capability_id(),
        method: NetworkMethod::Post,
        url: url.to_string(),
    }
}

fn wasm_request(
    request: &CredentialAccountResolverRequest,
    url: &str,
) -> WasmRuntimeCredentialRequest {
    wasm_request_with_method(request, request.method, url)
}

fn wasm_request_with_method(
    request: &CredentialAccountResolverRequest,
    method: NetworkMethod,
    url: &str,
) -> WasmRuntimeCredentialRequest {
    WasmRuntimeCredentialRequest {
        scope: request.scope.clone(),
        capability_id: request.capability_id.clone(),
        method,
        url: url.to_string(),
        headers: Vec::new(),
    }
}

fn sample_account(
    scope: &ResourceScope,
    id: &CredentialAccountId,
    secret_handle: &SecretHandle,
) -> CredentialAccount {
    CredentialAccount {
        scope: scope.clone(),
        id: id.clone(),
        provider_or_extension_id: extension_id(),
        label: "Telegram bot".to_string(),
        status: CredentialAccountStatus::Active,
        secret_handles: vec![secret_handle.clone()],
        allowed_targets: vec![CredentialTargetPolicy {
            scheme: "https".to_string(),
            host: "api.telegram.org".to_string(),
            port: Some(443),
            path: CredentialPathPolicy::Prefix("/bot123".to_string()),
            methods: vec![NetworkMethod::Post],
        }],
        redacted_metadata: RedactedJson::new(json!({ "kind": "bot_token" })),
        updated_at: Utc::now(),
    }
}

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").unwrap(),
        user_id: UserId::new("user-a").unwrap(),
        agent_id: Some(AgentId::new("agent-a").unwrap()),
        project_id: Some(ProjectId::new("project-a").unwrap()),
        mission_id: Some(MissionId::new("mission-a").unwrap()),
        thread_id: Some(ThreadId::new("thread-a").unwrap()),
        invocation_id: InvocationId::new(),
    }
}

fn extension_id() -> ExtensionId {
    ExtensionId::new("telegram").unwrap()
}

fn capability_id() -> CapabilityId {
    CapabilityId::new("telegram.send_message").unwrap()
}

struct MismatchedAccountStore {
    account: CredentialAccount,
}

#[async_trait]
impl ironclaw_secrets::CredentialAccountStore for MismatchedAccountStore {
    async fn put_account(
        &self,
        account: CredentialAccount,
    ) -> Result<CredentialAccount, CredentialBrokerError> {
        Ok(account)
    }

    async fn get_account(
        &self,
        _scope: &ResourceScope,
        _account_id: &CredentialAccountId,
    ) -> Result<Option<CredentialAccount>, CredentialBrokerError> {
        Ok(Some(self.account.clone()))
    }

    async fn accounts_for_scope(
        &self,
        _scope: &ResourceScope,
    ) -> Result<Vec<CredentialAccount>, CredentialBrokerError> {
        Ok(vec![self.account.clone()])
    }
}
