//! Composition root for the Reborn product runtime in this standalone host.
//!
//! Builds the durable storage stack + egress shim around the bot token. The
//! conversation-binding side routes through the **shared**
//! `ProductConversationBindingService` (introduced by PR #3727) instead of a
//! Telegram-specific table. The shared facade is fail-closed on unpaired
//! actors; this composition installs the operator-supplied
//! `REBORN_TELEGRAM_PAIRINGS` entries before returning so first-contact
//! inbound from a trusted user resolves cleanly.

use std::sync::Arc;

use ironclaw_host_api::{AgentId, TenantId, UserId};
use ironclaw_outbound::OutboundStateStore;
use ironclaw_product_adapters::{AdapterInstallationId, EgressCredentialHandle, ProductAdapterId};
use ironclaw_product_workflow::{
    ConversationBindingService, IdempotencyLedger, ProductConversationBindingService,
    ProductInstallationKey, ProductInstallationScope, StaticProductInstallationResolver,
};
use ironclaw_product_workflow_storage::{
    EgressCredentialResolver, StaticCredentialResolver, TelegramHttpEgress,
};

use crate::config::TelegramPairing;
use crate::error::HostError;

#[derive(Clone)]
pub struct RebornProductRuntime {
    pub ledger: Arc<dyn IdempotencyLedger>,
    pub binding: Arc<dyn ConversationBindingService>,
    pub outbound_store: Arc<dyn OutboundStateStore>,
    pub egress: Arc<TelegramHttpEgress>,
    pub default_tenant_id: TenantId,
    pub default_agent_id: AgentId,
}

type StorageLayer = (
    Arc<dyn IdempotencyLedger>,
    Arc<dyn ConversationBindingService>,
    Arc<dyn OutboundStateStore>,
);

pub struct RebornProductRuntimeConfig {
    pub default_tenant_id: TenantId,
    pub default_agent_id: AgentId,
    /// `ProductAdapterId` for this host. Used both as the conversations
    /// `AdapterKind` (the strings round-trip via `as_str`) and as the product
    /// installation resolver key.
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub telegram_bot_token: String,
    pub telegram_credential_handle: EgressCredentialHandle,
    pub telegram_declared_hosts: Vec<ironclaw_product_adapters::DeclaredEgressHost>,
    /// Trusted external-user → Reborn-user pairings to install before the
    /// runtime is returned. Each entry is validated as a `UserId` here; an
    /// invalid `user_id` fails composition closed so the host cannot start
    /// with a half-baked pairing table.
    pub pairings: Vec<TelegramPairing>,
}

/// Backend-specific handles. Exactly one variant is active; the crate's
/// top-level `connect_backend` helper constructs the matching variant from
/// env-resolved config.
pub enum BackendHandles {
    #[cfg(feature = "libsql")]
    LibSql(Arc<libsql::Database>),
    #[cfg(feature = "postgres")]
    Postgres(deadpool_postgres::Pool),
}

pub async fn build_reborn_product_runtime(
    handles: BackendHandles,
    config: RebornProductRuntimeConfig,
) -> Result<RebornProductRuntime, HostError> {
    let RebornProductRuntimeConfig {
        default_tenant_id,
        default_agent_id,
        adapter_id,
        installation_id,
        telegram_bot_token,
        telegram_credential_handle,
        telegram_declared_hosts,
        pairings,
    } = config;

    let installations = StaticProductInstallationResolver::new([(
        ProductInstallationKey::new(adapter_id.clone(), installation_id.clone()),
        ProductInstallationScope::with_default_scope(
            default_tenant_id.clone(),
            default_agent_id.clone(),
            None,
        ),
    )]);

    let (ledger, binding, outbound_store): StorageLayer = match handles {
        #[cfg(feature = "libsql")]
        BackendHandles::LibSql(db) => {
            build_libsql_layer(
                db,
                installations,
                &default_tenant_id,
                &adapter_id,
                &installation_id,
                &pairings,
            )
            .await?
        }
        #[cfg(feature = "postgres")]
        BackendHandles::Postgres(pool) => {
            build_postgres_layer(
                pool,
                installations,
                &default_tenant_id,
                &adapter_id,
                &installation_id,
                &pairings,
            )
            .await?
        }
    };

    let credentials: Arc<dyn EgressCredentialResolver> = Arc::new(StaticCredentialResolver::new(
        telegram_credential_handle.clone(),
        telegram_bot_token,
    ));
    let declared_targets: Vec<ironclaw_product_adapters::DeclaredEgressTarget> =
        telegram_declared_hosts
            .into_iter()
            .map(|host| {
                ironclaw_product_adapters::DeclaredEgressTarget::new(
                    host,
                    Some(telegram_credential_handle.clone()),
                )
            })
            .collect();
    let egress = TelegramHttpEgress::new(declared_targets, credentials)
        .map_err(|e| HostError::Startup(format!("egress client build: {e}")))?;

    Ok(RebornProductRuntime {
        ledger,
        binding,
        outbound_store,
        egress: Arc::new(egress),
        default_tenant_id,
        default_agent_id,
    })
}

/// Build the single-tenant fixed [`MountView`] this host owns. The standalone
/// Reborn binary runs one bot per process, so each alias resolves to itself
/// rather than a per-invocation tenant/user rewrite. Once the Reborn agent
/// loop and per-user scoping land, this should move to
/// `ironclaw_reborn_composition::invocation_mount_view` (which routes the
/// same aliases through tenant/user prefixes).
#[cfg(any(feature = "libsql", feature = "postgres"))]
fn fixed_host_mount_view() -> Result<ironclaw_host_api::MountView, HostError> {
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

    let aliases = ["/threads", "/outbound", "/conversations"];
    let mut grants = Vec::with_capacity(aliases.len());
    for alias in aliases {
        grants.push(MountGrant::new(
            MountAlias::new(alias)
                .map_err(|e| HostError::Startup(format!("{alias} mount alias: {e}")))?,
            VirtualPath::new(alias)
                .map_err(|e| HostError::Startup(format!("{alias} mount path: {e}")))?,
            MountPermissions::read_write_list_delete(),
        ));
    }
    MountView::new(grants).map_err(|e| HostError::Startup(format!("host mount view: {e}")))
}

/// Apply operator-supplied pairings to the conversations service. Each entry
/// is validated and then registered idempotently — `try_pair_external_actor`
/// is a no-op for an already-paired actor, so restarting the host with the
/// same env var does not error.
#[cfg(any(feature = "libsql", feature = "postgres"))]
async fn install_pairings<F>(
    conversations: &ironclaw_conversations::RebornFilesystemConversationServices,
    tenant_id: &TenantId,
    adapter_id: &ProductAdapterId,
    installation_id: &AdapterInstallationId,
    pairings: &[TelegramPairing],
    _backend_marker: std::marker::PhantomData<F>,
) -> Result<(), HostError>
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
{
    if pairings.is_empty() {
        return Ok(());
    }
    let adapter_kind = ironclaw_conversations::AdapterKind::new(adapter_id.as_str())
        .map_err(|e| HostError::Startup(format!("conversations adapter kind: {e}")))?;
    let conv_installation =
        ironclaw_conversations::AdapterInstallationId::new(installation_id.as_str())
            .map_err(|e| HostError::Startup(format!("conversations installation id: {e}")))?;
    for (idx, pairing) in pairings.iter().enumerate() {
        let canonical = UserId::new(&pairing.user_id).map_err(|e| {
            HostError::Startup(format!(
                "REBORN_TELEGRAM_PAIRINGS entry {idx} has invalid user_id {:?}: {e}",
                pairing.user_id
            ))
        })?;
        let actor = ironclaw_conversations::ExternalActorRef::new(
            ironclaw_telegram_v2_adapter::TELEGRAM_USER_ACTOR_KIND,
            &pairing.external_user_id,
        )
        .map_err(|e| {
            HostError::Startup(format!(
                "REBORN_TELEGRAM_PAIRINGS entry {idx} has invalid external_user_id {:?}: {e}",
                pairing.external_user_id
            ))
        })?;
        conversations
            .pair_external_actor(
                tenant_id.clone(),
                adapter_kind.clone(),
                conv_installation.clone(),
                actor,
                canonical,
            )
            .await
            .map_err(|e| {
                HostError::Startup(format!(
                    "pair_external_actor failed for entry {idx} (external_user_id={:?}): {e}",
                    pairing.external_user_id
                ))
            })?;
    }
    tracing::info!(
        count = pairings.len(),
        adapter_id = adapter_id.as_str(),
        installation_id = installation_id.as_str(),
        "Reborn host: installed REBORN_TELEGRAM_PAIRINGS entries"
    );
    Ok(())
}

#[cfg(feature = "libsql")]
async fn build_libsql_layer(
    db: Arc<libsql::Database>,
    installations: StaticProductInstallationResolver,
    default_tenant_id: &TenantId,
    adapter_id: &ProductAdapterId,
    installation_id: &AdapterInstallationId,
    pairings: &[TelegramPairing],
) -> Result<StorageLayer, HostError> {
    use ironclaw_conversations::RebornFilesystemConversationServices;
    use ironclaw_filesystem::{LibSqlRootFilesystem, ScopedFilesystem};
    use ironclaw_outbound::FilesystemOutboundStateStore;
    use ironclaw_product_workflow_storage::LibSqlProductIdempotencyLedger;

    let filesystem = Arc::new(LibSqlRootFilesystem::new(Arc::clone(&db)));
    filesystem
        .run_migrations()
        .await
        .map_err(|e| HostError::Storage(format!("filesystem migrations: {e}")))?;
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        filesystem,
        fixed_host_mount_view()?,
    ));
    let conversations = Arc::new(
        RebornFilesystemConversationServices::new(Arc::clone(&scoped))
            .await
            .map_err(|e| HostError::Storage(format!("conversations init: {e}")))?,
    );
    install_pairings::<LibSqlRootFilesystem>(
        &conversations,
        default_tenant_id,
        adapter_id,
        installation_id,
        pairings,
        std::marker::PhantomData,
    )
    .await?;

    let outbound: Arc<dyn OutboundStateStore> =
        Arc::new(FilesystemOutboundStateStore::new(Arc::clone(&scoped)));
    let ledger = Arc::new(LibSqlProductIdempotencyLedger::new(Arc::clone(&db)));
    let binding: Arc<dyn ConversationBindingService> = Arc::new(
        ProductConversationBindingService::new(conversations, installations),
    );
    Ok((ledger, binding, outbound))
}

#[cfg(feature = "postgres")]
async fn build_postgres_layer(
    pool: deadpool_postgres::Pool,
    installations: StaticProductInstallationResolver,
    default_tenant_id: &TenantId,
    adapter_id: &ProductAdapterId,
    installation_id: &AdapterInstallationId,
    pairings: &[TelegramPairing],
) -> Result<StorageLayer, HostError> {
    use ironclaw_conversations::RebornFilesystemConversationServices;
    use ironclaw_filesystem::{PostgresRootFilesystem, ScopedFilesystem};
    use ironclaw_outbound::FilesystemOutboundStateStore;
    use ironclaw_product_workflow_storage::PostgresProductIdempotencyLedger;

    let filesystem = Arc::new(PostgresRootFilesystem::new(pool.clone()));
    filesystem
        .run_migrations()
        .await
        .map_err(|e| HostError::Storage(format!("filesystem migrations: {e}")))?;
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        filesystem,
        fixed_host_mount_view()?,
    ));
    let conversations = Arc::new(
        RebornFilesystemConversationServices::new(Arc::clone(&scoped))
            .await
            .map_err(|e| HostError::Storage(format!("conversations init: {e}")))?,
    );
    install_pairings::<PostgresRootFilesystem>(
        &conversations,
        default_tenant_id,
        adapter_id,
        installation_id,
        pairings,
        std::marker::PhantomData,
    )
    .await?;

    let outbound: Arc<dyn OutboundStateStore> =
        Arc::new(FilesystemOutboundStateStore::new(Arc::clone(&scoped)));
    let ledger = Arc::new(PostgresProductIdempotencyLedger::new(pool.clone()));
    let binding: Arc<dyn ConversationBindingService> = Arc::new(
        ProductConversationBindingService::new(conversations, installations),
    );
    Ok((ledger, binding, outbound))
}
