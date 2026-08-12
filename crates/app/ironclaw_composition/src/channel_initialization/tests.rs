use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_assistant::DeliveryClientBootstrap as _;
use ironclaw_extension_contracts::channel_adapter::ChannelSurfaces;
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_host_api::{
    ids::{ExtensionId, InvocationId, SecretHandle, TenantId, UserId},
    resource::ResourceScope,
};
use ironclaw_secrets::SecretStore;
use secrecy::ExposeSecret as _;

use crate::{
    ChannelExtensionBinding, FirstPartyChannelInitializationContext,
    FirstPartyChannelInitializationError, FirstPartyChannelInitializer,
};

struct StaticInitializer {
    result: Result<Option<serde_json::Value>, FirstPartyChannelInitializationError>,
}

struct GeneratedInitializer;

#[async_trait]
impl FirstPartyChannelInitializer for GeneratedInitializer {
    async fn initialize(
        &self,
        context: &FirstPartyChannelInitializationContext<'_>,
    ) -> Result<Option<serde_json::Value>, FirstPartyChannelInitializationError> {
        let handle = SecretHandle::new("replica_safe_generated_key")
            .map_err(|error| FirstPartyChannelInitializationError::failed(error.to_string()))?;
        context
            .store_credential_if_absent(handle.clone(), uuid::Uuid::new_v4().to_string())
            .await?;
        let winner = context.read_credential_once(&handle).await?;
        Ok(Some(
            serde_json::json!({ "winner": winner.expose_secret() }),
        ))
    }
}

#[async_trait]
impl FirstPartyChannelInitializer for StaticInitializer {
    async fn initialize(
        &self,
        _context: &FirstPartyChannelInitializationContext<'_>,
    ) -> Result<Option<serde_json::Value>, FirstPartyChannelInitializationError> {
        self.result.clone()
    }
}

fn scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
        user_id: UserId::new("operator").expect("user"),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn binding(
    extension_id: &str,
    initializer: Arc<dyn FirstPartyChannelInitializer>,
) -> ChannelExtensionBinding {
    ChannelExtensionBinding {
        extension_id: ExtensionId::new(extension_id).expect("extension id"),
        surfaces: ChannelSurfaces::default(),
        preference_target_codec: None,
        outbound_target_provider: None,
        first_party_initializer: Some(initializer),
        registration_document_path: None,
    }
}

#[tokio::test]
async fn binding_initializer_publishes_bootstrap_by_typed_extension_id() {
    let store = SecretStore::ephemeral_over(Arc::new(InMemoryBackend::new()));
    let bindings = vec![binding(
        "channel-a",
        Arc::new(StaticInitializer {
            result: Ok(Some(serde_json::json!({ "public_key": "pk-a" }))),
        }),
    )];

    let bootstraps = super::initialize_first_party_channels(&bindings, &store, scope())
        .await
        .expect("initializer succeeds");

    assert_eq!(
        bootstraps
            .bootstrap("channel-a")
            .expect("bootstrap lookup succeeds"),
        Some(serde_json::json!({ "public_key": "pk-a" }))
    );
    assert_eq!(
        bootstraps
            .bootstrap("channel-b")
            .expect("unknown lookup succeeds"),
        None
    );
}

#[tokio::test]
async fn binding_initializer_failure_aborts_bootstrap_assembly() {
    let store = SecretStore::ephemeral_over(Arc::new(InMemoryBackend::new()));
    let bindings = vec![binding(
        "channel-a",
        Arc::new(StaticInitializer {
            result: Err(FirstPartyChannelInitializationError::failed(
                "bootstrap unavailable",
            )),
        }),
    )];

    let error = super::initialize_first_party_channels(&bindings, &store, scope())
        .await
        .expect_err("initializer failure must fail assembly");

    assert!(error.to_string().contains("channel-a"));
}

#[tokio::test]
async fn concurrent_initializers_publish_the_same_generated_credential() {
    let store = SecretStore::ephemeral_over(Arc::new(InMemoryBackend::new()));
    let first_bindings = vec![binding("channel-a", Arc::new(GeneratedInitializer))];
    let second_bindings = vec![binding("channel-a", Arc::new(GeneratedInitializer))];
    let first = super::initialize_first_party_channels(&first_bindings, &store, scope());
    let second = super::initialize_first_party_channels(&second_bindings, &store, scope());
    let (first, second) = tokio::join!(first, second);

    assert_eq!(
        first.expect("first initializer").bootstrap("channel-a"),
        second.expect("second initializer").bootstrap("channel-a"),
        "every replica must publish bootstrap data from the one winning secret",
    );
}
