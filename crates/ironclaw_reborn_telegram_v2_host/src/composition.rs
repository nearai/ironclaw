//! Composition root for the Reborn product runtime in this standalone host.
//!
//! Builds the durable storage stack + egress shim around the bot token.
//! Mirrors the file that used to live in `src/channels/reborn/composition.rs`
//! but takes concrete DB handles directly instead of v1's `DatabaseHandles`,
//! and returns this crate's local `HostError` instead of v1's `ChannelError`.

use std::sync::Arc;

use ironclaw_host_api::{AgentId, TenantId};
use ironclaw_outbound::OutboundStateStore;
use ironclaw_product_adapters::EgressCredentialHandle;
use ironclaw_product_workflow::{ConversationBindingService, IdempotencyLedger};
use ironclaw_product_workflow_storage::{
    EgressCredentialResolver, StaticCredentialResolver, TelegramHttpEgress,
};
use ironclaw_threads::SessionThreadService;

use crate::error::HostError;

#[derive(Clone)]
pub struct RebornProductRuntime {
    pub ledger: Arc<dyn IdempotencyLedger>,
    pub binding: Arc<dyn ConversationBindingService>,
    pub outbound_store: Arc<dyn OutboundStateStore>,
    pub egress: Arc<TelegramHttpEgress>,
    pub thread_service: Arc<dyn SessionThreadService>,
    pub default_tenant_id: TenantId,
    pub default_agent_id: AgentId,
}

type StorageLayer = (
    Arc<dyn IdempotencyLedger>,
    Arc<dyn ConversationBindingService>,
    Arc<dyn OutboundStateStore>,
    Arc<dyn SessionThreadService>,
);

pub struct RebornProductRuntimeConfig {
    pub default_tenant_id: TenantId,
    pub default_agent_id: AgentId,
    pub telegram_bot_token: String,
    pub telegram_credential_handle: EgressCredentialHandle,
    pub telegram_declared_hosts: Vec<ironclaw_product_adapters::DeclaredEgressHost>,
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
        telegram_bot_token,
        telegram_credential_handle,
        telegram_declared_hosts,
    } = config;

    let (ledger, binding, outbound_store, thread_service): StorageLayer = match handles {
        #[cfg(feature = "libsql")]
        BackendHandles::LibSql(db) => {
            build_libsql_layer(db, &default_tenant_id, &default_agent_id).await?
        }
        #[cfg(feature = "postgres")]
        BackendHandles::Postgres(pool) => {
            build_postgres_layer(pool, &default_tenant_id, &default_agent_id).await?
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
        thread_service,
        default_tenant_id,
        default_agent_id,
    })
}

#[cfg(feature = "libsql")]
async fn build_libsql_layer(
    db: Arc<libsql::Database>,
    default_tenant_id: &TenantId,
    default_agent_id: &AgentId,
) -> Result<StorageLayer, HostError> {
    use ironclaw_outbound::LibSqlOutboundStateStore;
    use ironclaw_product_workflow_storage::{
        LibSqlConversationBindingService, LibSqlProductIdempotencyLedger,
    };
    use ironclaw_threads::LibSqlSessionThreadService;

    let thread_service_concrete = LibSqlSessionThreadService::new(Arc::clone(&db));
    thread_service_concrete
        .run_migrations()
        .await
        .map_err(|e| HostError::Storage(format!("thread service migrations: {e}")))?;
    let thread_service: Arc<dyn SessionThreadService> = Arc::new(thread_service_concrete);

    let outbound = LibSqlOutboundStateStore::new(Arc::clone(&db));
    outbound
        .run_migrations()
        .await
        .map_err(|e| HostError::Storage(format!("outbound migrations: {e}")))?;

    let ledger = Arc::new(LibSqlProductIdempotencyLedger::new(Arc::clone(&db)));
    let binding = Arc::new(LibSqlConversationBindingService::new(
        Arc::clone(&db),
        Arc::clone(&thread_service),
        default_tenant_id.clone(),
        default_agent_id.clone(),
    ));
    Ok((ledger, binding, Arc::new(outbound), thread_service))
}

#[cfg(feature = "postgres")]
async fn build_postgres_layer(
    pool: deadpool_postgres::Pool,
    default_tenant_id: &TenantId,
    default_agent_id: &AgentId,
) -> Result<StorageLayer, HostError> {
    use ironclaw_outbound::PostgresOutboundStateStore;
    use ironclaw_product_workflow_storage::{
        PostgresConversationBindingService, PostgresProductIdempotencyLedger,
    };
    use ironclaw_threads::PostgresSessionThreadService;

    let thread_service_concrete = PostgresSessionThreadService::new(pool.clone());
    thread_service_concrete
        .run_migrations()
        .await
        .map_err(|e| HostError::Storage(format!("thread service migrations: {e}")))?;
    let thread_service: Arc<dyn SessionThreadService> = Arc::new(thread_service_concrete);

    let outbound = PostgresOutboundStateStore::new(pool.clone());
    outbound
        .run_migrations()
        .await
        .map_err(|e| HostError::Storage(format!("outbound migrations: {e}")))?;

    let ledger = Arc::new(PostgresProductIdempotencyLedger::new(pool.clone()));
    let binding = Arc::new(PostgresConversationBindingService::new(
        pool.clone(),
        Arc::clone(&thread_service),
        default_tenant_id.clone(),
        default_agent_id.clone(),
    ));
    Ok((ledger, binding, Arc::new(outbound), thread_service))
}
