//! The operator LLM key store wired the way production `ironclaw serve` wires
//! it — the dynamic `invocation_mount_view` scoped filesystem behind a real
//! `SecretStore`, behind the `OperatorSecretValueStore` adapter this crate
//! implements.
//!
//! ✎ **Relocated here by WS3** from
//! `ironclaw_operator::llm_admin::llm_config_service`'s test module. The test
//! below is the #4673 reproduction, and its entire value is that it builds the
//! *production* store rather than the in-memory one the operator's other tests
//! use. After WS3 removed `ironclaw_operator`'s direct `ironclaw_secrets` edge
//! (PROPOSAL §8.2's product row, §12.1b), "the production store" is a real
//! `SecretStore` **plus** `RuntimeOperatorSecretValueStore` — a combination only
//! the assembly layer can name. Keeping the test in `ironclaw_operator` would
//! have meant pointing it at a fake, which is exactly the fidelity it exists to
//! have. It travelled whole; no assertion was weakened.

use std::sync::Arc;

use ironclaw_composition::RuntimeOperatorSecretValueStore;
use ironclaw_config::{RebornBootConfig, RebornHome, RebornProfile};
use ironclaw_filesystem::{InMemoryBackend, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    ids::{AgentId, ProjectId, TenantId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
};
use ironclaw_operator::{LlmKeyStore, llm_admin::llm_config_service::RebornLlmConfigService};
use ironclaw_product_contracts::{
    operator_llm::{LlmConfigService, UpsertLlmProviderRequest},
    surface::ProductSurfaceCaller,
};
use ironclaw_secrets::{SecretMaterial, SecretStore, SecretsCrypto};
use secrecy::{ExposeSecret, SecretString};

fn boot_for_home(reborn_home: &std::path::Path) -> RebornBootConfig {
    let home = RebornHome::resolve_from_env_parts(
        Some(reborn_home.as_os_str().to_os_string()),
        None,
        None,
    )
    .expect("valid reborn home");
    RebornBootConfig::new(home, RebornProfile::Standalone)
}

/// The `/secrets` mount production assembles for the secret store.
fn secret_store_scoped<F>(root: Arc<F>) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    Arc::new(ScopedFilesystem::with_fixed_view(
        root,
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/secrets").expect("valid secrets alias"),
            VirtualPath::new("/engine/secrets").expect("valid secrets target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("valid secrets mount"),
    ))
}

fn caller() -> ProductSurfaceCaller {
    ProductSurfaceCaller::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        UserId::new("user-alpha").expect("user"),
        Some(AgentId::new("agent-alpha").expect("agent")),
        Some(ProjectId::new("project-alpha").expect("project")),
    )
}

/// Reproduction for issue #4673: saving the NEAR AI (builtin) provider returns
/// `service_unavailable` even though Test connection succeeds. Wires the secret
/// store EXACTLY as production `ironclaw serve` does, so a system-scope
/// write/read regression in that path is caught.
#[tokio::test]
async fn upsert_builtin_nearai_with_production_secret_store_succeeds() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let boot = boot_for_home(&reborn_home);

    let backend = Arc::new(InMemoryBackend::default());
    let scoped = secret_store_scoped(backend);
    let crypto = Arc::new(
        SecretsCrypto::new(SecretMaterial::from(
            "0123456789abcdef0123456789abcdef".to_string(),
        ))
        .expect("valid master key"),
    );
    let keys = LlmKeyStore::new(RuntimeOperatorSecretValueStore::shared(Arc::new(
        SecretStore::new(scoped, crypto),
    )));

    let nearai_request = || UpsertLlmProviderRequest {
        id: "nearai".to_string(),
        client_action_id: None,
        name: Some("NEAR AI".to_string()),
        adapter: "near_ai".to_string(),
        base_url: Some("https://cloud-api.near.ai".to_string()),
        default_model: Some("deepseek-ai/DeepSeek-V4-Flash".to_string()),
        api_key: Some(SecretString::from("sk-near-test")),
        set_active: true,
        model: Some("deepseek-ai/DeepSeek-V4-Flash".to_string()),
    };

    let service = RebornLlmConfigService::new(boot.clone(), keys.clone());
    // First save persists the operator's NEAR AI key under the system scope.
    let snapshot = service
        .upsert_provider(caller(), nearai_request())
        .await
        .expect("saving the builtin NEAR AI provider must succeed");
    let active = snapshot.active.expect("an active provider after save");
    assert_eq!(active.provider_id, "nearai");
    assert_eq!(
        active.model.as_deref(),
        Some("deepseek-ai/DeepSeek-V4-Flash")
    );

    // The stored system-scoped key must read back (the #4673 regression: the
    // reserved system tenant id failed to deserialize, so any read-back of a
    // system-scoped secret errored — including a second save, which reads the
    // previous key first).
    assert_eq!(
        keys.read("nearai")
            .await
            .expect("system-scope key must read back")
            .expect("a stored key")
            .expose_secret(),
        "sk-near-test"
    );
    service
        .upsert_provider(caller(), nearai_request())
        .await
        .expect("re-saving an already-configured NEAR AI provider must succeed");
}
