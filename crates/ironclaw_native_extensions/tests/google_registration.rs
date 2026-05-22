use std::sync::Arc;

use ironclaw_host_api::{InvocationId, ResourceScope, SecretHandle, UserId};
use ironclaw_native_extensions::google::client::{
    GoogleApiErrorKind, GoogleHttpClient, map_google_response,
};
use ironclaw_native_extensions::google::credential::{
    GOOGLE_CREDENTIAL_NAME, GoogleCredentialResolver,
};
use ironclaw_native_extensions::google::oauth_provider::BAKED_IN_GOOGLE_DESKTOP_CLIENT_ID;
use ironclaw_native_extensions::{EnvConfig, register_all};
use ironclaw_network::{NetworkHttpResponse, NetworkUsage};
use ironclaw_oauth::{TokenPersister, TokenSet};
use ironclaw_secrets::{InMemorySecretStore, SecretStore};
use secrecy::ExposeSecret;
use serde_json::json;

#[test]
fn google_provider_registers_in_broker_mode_with_baked_client_id() {
    let output = register_all(
        &EnvConfig {
            oauth_broker_active: true,
            google_client_id: None,
            google_client_secret: None,
            google_allowed_hd: Some("example.com".to_string()),
        },
        Arc::new(InMemorySecretStore::new()),
    )
    .unwrap();

    assert_eq!(output.oauth_providers.len(), 1);
    let provider = &output.oauth_providers[0];
    assert_eq!(provider.provider_id(), "google");
    assert_eq!(
        provider.public_client_id(),
        BAKED_IN_GOOGLE_DESKTOP_CLIENT_ID
    );
    assert!(provider.direct_client_secret().is_none());
    assert_eq!(output.network_policies.len(), 1);
    // The Google Calendar package (nine capabilities) and the Gmail package
    // (six capabilities) register alongside the provider once Google is
    // enabled — two packages and fifteen capability handlers in total.
    assert_eq!(output.packages.len(), 2);
    assert_eq!(output.packages[0].id.as_str(), "google-calendar");
    assert_eq!(output.packages[0].capabilities.len(), 9);
    assert_eq!(output.packages[1].id.as_str(), "gmail");
    assert_eq!(output.packages[1].capabilities.len(), 6);
    assert_eq!(output.handlers.len(), 15);

    let url = provider.build_authorize_url(
        "state",
        "challenge",
        &["scope-a".to_string()],
        "https://app.example.test/oauth/callback",
    );
    assert!(url.contains("include_granted_scopes=true"));
    assert!(url.contains("hd=example.com"));
}

#[test]
fn google_provider_registers_in_direct_mode_with_runtime_secret() {
    let output = register_all(
        &EnvConfig {
            oauth_broker_active: false,
            google_client_id: Some("direct-client".to_string()),
            google_client_secret: Some("direct-secret".to_string()),
            google_allowed_hd: None,
        },
        Arc::new(InMemorySecretStore::new()),
    )
    .unwrap();

    let provider = &output.oauth_providers[0];
    assert_eq!(provider.public_client_id(), "direct-client");
    assert_eq!(
        provider.direct_client_secret().unwrap().expose_secret(),
        "direct-secret"
    );
}

#[test]
fn google_provider_is_disabled_in_direct_mode_without_client_id() {
    let output = register_all(
        &EnvConfig {
            oauth_broker_active: false,
            google_client_id: None,
            google_client_secret: Some("direct-secret".to_string()),
            google_allowed_hd: None,
        },
        Arc::new(InMemorySecretStore::new()),
    )
    .unwrap();

    assert!(output.oauth_providers.is_empty());
    assert!(output.network_policies.is_empty());
}

#[tokio::test]
async fn credential_resolver_reports_scope_mismatch_and_refs() {
    let secrets = Arc::new(InMemorySecretStore::new());
    let resolver = GoogleCredentialResolver::new(secrets.clone());
    let scope = sample_scope();
    TokenPersister::new(secrets)
        .persist(
            &scope,
            GOOGLE_CREDENTIAL_NAME,
            &TokenSet::from_expires_in(
                "access-token",
                Some("refresh-token".to_string()),
                Some(3600),
                vec!["scope-a".to_string()],
            ),
        )
        .await
        .unwrap();
    let output = register_all(
        &EnvConfig {
            oauth_broker_active: true,
            google_client_id: None,
            google_client_secret: None,
            google_allowed_hd: None,
        },
        Arc::new(InMemorySecretStore::new()),
    )
    .unwrap();
    let provider = output.oauth_providers[0].as_ref();

    let credential = resolver
        .resolve(
            &scope,
            provider,
            &["scope-a".to_string(), "scope-b".to_string()],
        )
        .await
        .unwrap();
    assert_eq!(credential.access_token.expose_secret(), "access-token");
    assert_eq!(credential.missing_scopes, vec!["scope-b"]);
    assert!(!credential.refresh_required);

    let extension = ironclaw_host_api::ExtensionId::new("google-calendar").unwrap();
    assert_eq!(
        resolver.add_ref(&scope, &extension).await.unwrap(),
        vec![extension.clone()]
    );
    assert_eq!(
        resolver.add_ref(&scope, &extension).await.unwrap(),
        vec![extension.clone()]
    );
    assert!(
        resolver
            .remove_ref(&scope, &extension)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        resolver
            .load_refs(&scope)
            .await
            .expect("refs row should be deleted as empty")
            .is_empty()
    );
    let access_token_handle = SecretHandle::new(GOOGLE_CREDENTIAL_NAME).unwrap();
    assert!(
        resolver
            .remove_ref(&scope, &extension)
            .await
            .expect("re-removing is idempotent")
            .is_empty()
    );
    assert!(
        resolver
            .load_refs(&scope)
            .await
            .expect("missing refs row is empty")
            .is_empty()
    );
    let cleanup_secrets = Arc::new(InMemorySecretStore::new());
    let cleanup_resolver = GoogleCredentialResolver::new(cleanup_secrets.clone());
    TokenPersister::new(cleanup_secrets.clone())
        .persist(
            &scope,
            GOOGLE_CREDENTIAL_NAME,
            &TokenSet::from_expires_in(
                "access-token",
                Some("refresh-token".to_string()),
                Some(3600),
                vec!["scope-a".to_string()],
            ),
        )
        .await
        .unwrap();
    cleanup_resolver.add_ref(&scope, &extension).await.unwrap();
    cleanup_resolver
        .remove_ref(&scope, &extension)
        .await
        .unwrap();
    assert!(
        cleanup_secrets
            .metadata(&scope, &access_token_handle)
            .await
            .unwrap()
            .is_none()
    );
}

#[test]
fn google_api_error_mapper_marks_refresh_and_auth_prompt_cases() {
    let unauthorized =
        map_google_response(response(401, json!({"error": "invalid_token"}))).unwrap_err();
    assert_eq!(unauthorized.kind, GoogleApiErrorKind::RefreshRequired);
    assert!(unauthorized.should_refresh());

    let insufficient = map_google_response(response(
        403,
        json!({"error": {"status": "INSUFFICIENT_SCOPE"}}),
    ))
    .unwrap_err();
    assert_eq!(insufficient.kind, GoogleApiErrorKind::InsufficientScope);
    assert!(insufficient.requires_auth_prompt());

    let nested_insufficient = map_google_response(response(
        403,
        json!({
            "error": {
                "status": "PERMISSION_DENIED",
                "details": [{"reason": "insufficient_scope"}]
            }
        }),
    ))
    .unwrap_err();
    assert_eq!(
        nested_insufficient.kind,
        GoogleApiErrorKind::InsufficientScope
    );
    assert!(nested_insufficient.requires_auth_prompt());

    let quota_exceeded = map_google_response(response(
        403,
        json!({"error": {"status": "PERMISSION_DENIED", "message": "quota exceeded"}}),
    ))
    .unwrap_err();
    assert_eq!(quota_exceeded.kind, GoogleApiErrorKind::HttpStatus);
    assert!(!quota_exceeded.requires_auth_prompt());
}

fn response(status: u16, body: serde_json::Value) -> NetworkHttpResponse {
    NetworkHttpResponse {
        status,
        headers: Vec::new(),
        body: serde_json::to_vec(&body).unwrap(),
        usage: NetworkUsage::default(),
    }
}

fn sample_scope() -> ResourceScope {
    ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap()
}

#[allow(dead_code)]
fn assert_google_client_is_constructible(egress: Arc<dyn ironclaw_network::NetworkHttpEgress>) {
    let _client = GoogleHttpClient::new(egress);
}
