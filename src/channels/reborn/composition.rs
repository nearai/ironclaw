//! Composition root for the Reborn product runtime (Telegram v2).
//!
//! Builds the storage/transport pieces that back a `ProductWorkflow` instance
//! for the v2 channel path. This intentionally lives in `src/` rather than in
//! `crates/ironclaw_reborn_composition/` so the bigger substrate composition
//! crate stays focused on substrate concerns and does not need to pull in
//! every product-layer crate.
//!
//! The runtime is constructed lazily at app boot when
//! `REBORN_TELEGRAM_V2_ENABLED=true`. The custom `InboundTurnService` that
//! bridges into the v1 agent runtime via `ChannelManager` lives in
//! [`super::v2_inbound_turn`]; this file only assembles the durable +
//! transport layer.

use std::sync::Arc;

use ironclaw_host_api::{AgentId, TenantId};
use ironclaw_outbound::OutboundStateStore;
use ironclaw_product_adapters::EgressCredentialHandle;
use ironclaw_product_workflow::{ConversationBindingService, IdempotencyLedger};
use ironclaw_product_workflow_storage::{
    EgressCredentialResolver, StaticCredentialResolver, TelegramHttpEgress,
};
use ironclaw_threads::SessionThreadService;

use crate::db::DatabaseHandles;
use crate::error::ChannelError;

/// Bundled handles for the Reborn product runtime, used by the v2 webhook
/// route and synthetic channel.
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

/// Bundle of the four storage handles a backend-specific builder returns.
type StorageLayer = (
    Arc<dyn IdempotencyLedger>,
    Arc<dyn ConversationBindingService>,
    Arc<dyn OutboundStateStore>,
    Arc<dyn SessionThreadService>,
);

/// Configuration carried into runtime construction. Mirrors the secret values
/// the operator must have configured before flipping the v2 flag.
pub struct RebornProductRuntimeConfig {
    pub default_tenant_id: TenantId,
    pub default_agent_id: AgentId,
    pub telegram_bot_token: String,
    pub telegram_credential_handle: EgressCredentialHandle,
    pub telegram_declared_hosts: Vec<ironclaw_product_adapters::DeclaredEgressHost>,
}

/// Build the runtime against whichever DB backend is configured.
///
/// Returns `Ok(None)` when no DB backend is available (defensive — should not
/// happen in practice since the host binary refuses to start without one).
pub async fn build_reborn_product_runtime(
    handles: &DatabaseHandles,
    config: RebornProductRuntimeConfig,
) -> Result<RebornProductRuntime, ChannelError> {
    let RebornProductRuntimeConfig {
        default_tenant_id,
        default_agent_id,
        telegram_bot_token,
        telegram_credential_handle,
        telegram_declared_hosts,
    } = config;

    // Build the durable storage stack against whichever DB backend is active.
    // Postgres takes precedence when both handles are present (matches v1
    // behavior elsewhere in the binary). All stores — including the
    // SessionThreadService that the binding service depends on — must be
    // backed by a real DB in production; an in-memory thread service would
    // lose thread IDs on restart and the binding rows would point at dangling
    // threads on next inbound.
    #[cfg(feature = "postgres")]
    let (ledger, binding, outbound_store, thread_service): StorageLayer =
        if let Some(pool) = handles.pg_pool.as_ref() {
            build_postgres_layer(pool, &default_tenant_id, &default_agent_id).await?
        } else {
            build_libsql_layer(handles, &default_tenant_id, &default_agent_id).await?
        };

    #[cfg(not(feature = "postgres"))]
    let (ledger, binding, outbound_store, thread_service) =
        build_libsql_layer(handles, &default_tenant_id, &default_agent_id).await?;

    // Egress shim — Telegram-specific, owns the bot token.
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
    let egress = TelegramHttpEgress::new(declared_targets, credentials).map_err(|e| {
        ChannelError::StartupFailed {
            name: "reborn_telegram_v2".into(),
            reason: format!("egress client build: {e}"),
        }
    })?;

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
    handles: &DatabaseHandles,
    default_tenant_id: &TenantId,
    default_agent_id: &AgentId,
) -> Result<StorageLayer, ChannelError> {
    use ironclaw_outbound::LibSqlOutboundStateStore;
    use ironclaw_product_workflow_storage::{
        LibSqlConversationBindingService, LibSqlProductIdempotencyLedger,
    };
    use ironclaw_threads::LibSqlSessionThreadService;

    let libsql_db = handles
        .libsql_db
        .as_ref()
        .ok_or_else(|| ChannelError::StartupFailed {
            name: "reborn_telegram_v2".into(),
            reason: "requires libSQL or Postgres backend".into(),
        })?;

    // SessionThreadService — durable across restarts. Owns its own schema.
    let thread_service_concrete = LibSqlSessionThreadService::new(Arc::clone(libsql_db));
    thread_service_concrete
        .run_migrations()
        .await
        .map_err(|e| ChannelError::StartupFailed {
            name: "reborn_telegram_v2".into(),
            reason: format!("thread service migrations: {e}"),
        })?;
    let thread_service: Arc<dyn SessionThreadService> = Arc::new(thread_service_concrete);

    let outbound = LibSqlOutboundStateStore::new(Arc::clone(libsql_db));
    outbound
        .run_migrations()
        .await
        .map_err(|e| ChannelError::StartupFailed {
            name: "reborn_telegram_v2".into(),
            reason: format!("outbound migrations: {e}"),
        })?;

    let ledger = Arc::new(LibSqlProductIdempotencyLedger::new(Arc::clone(libsql_db)));
    let binding = Arc::new(LibSqlConversationBindingService::new(
        Arc::clone(libsql_db),
        Arc::clone(&thread_service),
        default_tenant_id.clone(),
        default_agent_id.clone(),
    ));
    Ok((ledger, binding, Arc::new(outbound), thread_service))
}

#[cfg(not(feature = "libsql"))]
async fn build_libsql_layer(
    _handles: &DatabaseHandles,
    _default_tenant_id: &TenantId,
    _default_agent_id: &AgentId,
) -> Result<StorageLayer, ChannelError> {
    Err(ChannelError::StartupFailed {
        name: "reborn_telegram_v2".into(),
        reason: "requires libSQL or Postgres backend".into(),
    })
}

#[cfg(feature = "postgres")]
async fn build_postgres_layer(
    pool: &deadpool_postgres::Pool,
    default_tenant_id: &TenantId,
    default_agent_id: &AgentId,
) -> Result<StorageLayer, ChannelError> {
    use ironclaw_outbound::PostgresOutboundStateStore;
    use ironclaw_product_workflow_storage::{
        PostgresConversationBindingService, PostgresProductIdempotencyLedger,
    };
    use ironclaw_threads::PostgresSessionThreadService;

    let thread_service_concrete = PostgresSessionThreadService::new(pool.clone());
    thread_service_concrete
        .run_migrations()
        .await
        .map_err(|e| ChannelError::StartupFailed {
            name: "reborn_telegram_v2".into(),
            reason: format!("thread service migrations: {e}"),
        })?;
    let thread_service: Arc<dyn SessionThreadService> = Arc::new(thread_service_concrete);

    let outbound = PostgresOutboundStateStore::new(pool.clone());
    outbound
        .run_migrations()
        .await
        .map_err(|e| ChannelError::StartupFailed {
            name: "reborn_telegram_v2".into(),
            reason: format!("outbound migrations: {e}"),
        })?;

    let ledger = Arc::new(PostgresProductIdempotencyLedger::new(pool.clone()));
    let binding = Arc::new(PostgresConversationBindingService::new(
        pool.clone(),
        Arc::clone(&thread_service),
        default_tenant_id.clone(),
        default_agent_id.clone(),
    ));
    Ok((ledger, binding, Arc::new(outbound), thread_service))
}
