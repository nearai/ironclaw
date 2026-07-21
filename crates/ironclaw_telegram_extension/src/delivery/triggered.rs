//! Setup-revision-aware triggered-run delivery into Telegram DMs.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_channel_delivery::{
    PostSubmitDeliveryError, PostSubmitDeliveryHook, TriggeredRunDeliveryDriver,
};
use ironclaw_channel_host::outbound_targets::OutboundDeliveryTargetProvider;
use ironclaw_outbound::{
    DeliveredGateRouteStore, TriggeredRunDeliveryOutcomeKind, TriggeredRunDeliveryRecord,
    TriggeredRunDeliveryStore,
};
use ironclaw_product_workflow::{
    ConversationBindingService, ProductWorkflowError, ResolveBindingRequest, ResolvedBinding,
};
use ironclaw_triggers::TriggerFire;
use ironclaw_turns::{TurnRunId, TurnScope};
use tokio::sync::Mutex;

use crate::TelegramHostBuildError;
use crate::host::TelegramRevisionWorkflowParts;
use crate::setup::{TelegramInstallationSetup, TelegramSetupService};

/// Trigger hook that lazily builds and caches a driver for the current setup
/// revision. First setup and bot swaps take effect without a process restart.
pub struct DynamicTelegramTriggeredRunDeliveryHook {
    revision_parts: Arc<TelegramRevisionWorkflowParts>,
    setup_service: Arc<TelegramSetupService>,
    delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
    outbound_target_provider: Arc<dyn OutboundDeliveryTargetProvider>,
    cached_driver: RevisionCache<TriggeredRunDeliveryDriver>,
}

struct CachedRevision<T> {
    revision: u64,
    value: Arc<T>,
}

struct RevisionCache<T> {
    cached: Mutex<Option<CachedRevision<T>>>,
}

impl<T> Default for RevisionCache<T> {
    fn default() -> Self {
        Self {
            cached: Mutex::new(None),
        }
    }
}

impl<T> RevisionCache<T> {
    async fn get(&self, revision: u64) -> Option<Arc<T>> {
        let cached = self.cached.lock().await;
        cached
            .as_ref()
            .filter(|cached| cached.revision == revision)
            .map(|cached| Arc::clone(&cached.value))
    }

    async fn install_unless_newer(&self, revision: u64, candidate: Arc<T>) -> Arc<T> {
        let mut cached = self.cached.lock().await;
        if let Some(current) = cached
            .as_ref()
            .filter(|current| current.revision >= revision)
        {
            return Arc::clone(&current.value);
        }
        *cached = Some(CachedRevision {
            revision,
            value: Arc::clone(&candidate),
        });
        candidate
    }
}

impl DynamicTelegramTriggeredRunDeliveryHook {
    pub(crate) fn new(
        revision_parts: Arc<TelegramRevisionWorkflowParts>,
        setup_service: Arc<TelegramSetupService>,
        delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
        outbound_target_provider: Arc<dyn OutboundDeliveryTargetProvider>,
    ) -> Self {
        Self {
            revision_parts,
            setup_service,
            delivery_store,
            outbound_target_provider,
            cached_driver: RevisionCache::default(),
        }
    }

    async fn current_driver(&self) -> Result<Option<Arc<TriggeredRunDeliveryDriver>>, String> {
        let Some(setup) = self
            .setup_service
            .current_setup()
            .await
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        let revision = setup.revision;

        if let Some(driver) = self.cached_driver.get(revision).await {
            return Ok(Some(driver));
        }

        let driver = self
            .build_driver(&setup)
            .map_err(|error| error.to_string())?;
        Ok(Some(
            self.cached_driver
                .install_unless_newer(revision, driver)
                .await,
        ))
    }

    fn build_driver(
        &self,
        setup: &TelegramInstallationSetup,
    ) -> Result<Arc<TriggeredRunDeliveryDriver>, TelegramHostBuildError> {
        let installation_id = setup
            .installation_id()
            .map_err(|reason| invalid_config("installation_id", reason.to_string()))?;
        let adapter = self
            .revision_parts
            .adapter_for_setup(setup, installation_id)?;
        let route_store: Arc<dyn DeliveredGateRouteStore> =
            self.revision_parts.delivered_gate_routes();
        let driver = TriggeredRunDeliveryDriver::new(
            self.revision_parts.final_reply_delivery_services(
                Arc::new(NoopTelegramConversationBindingService),
                adapter,
            ),
            Arc::clone(&self.delivery_store),
            route_store,
            self.revision_parts.config().agent_id.clone(),
        )
        .with_outbound_target_provider(Arc::clone(&self.outbound_target_provider));
        Ok(Arc::new(driver))
    }
}

#[async_trait]
impl PostSubmitDeliveryHook for DynamicTelegramTriggeredRunDeliveryHook {
    async fn on_trigger_submitted(
        &self,
        fire: TriggerFire,
        run_id: TurnRunId,
        scope: TurnScope,
    ) -> Result<(), PostSubmitDeliveryError> {
        match self.current_driver().await {
            Ok(Some(driver)) => {
                PostSubmitDeliveryHook::on_trigger_submitted(driver.as_ref(), fire, run_id, scope)
                    .await
            }
            Ok(None) => {
                tracing::debug!(
                    %run_id,
                    "Telegram triggered-run delivery skipped: Telegram setup is not configured"
                );
                self.record_terminal_outcome(run_id, TriggeredRunDeliveryOutcomeKind::Skipped)
                    .await
            }
            Err(error) => {
                tracing::debug!(
                    %run_id,
                    %error,
                    "Telegram triggered-run delivery skipped: delivery hook unavailable"
                );
                self.record_terminal_outcome(run_id, TriggeredRunDeliveryOutcomeKind::Failed)
                    .await
            }
        }
    }
}

impl DynamicTelegramTriggeredRunDeliveryHook {
    async fn record_terminal_outcome(
        &self,
        run_id: TurnRunId,
        outcome: TriggeredRunDeliveryOutcomeKind,
    ) -> Result<(), PostSubmitDeliveryError> {
        let record = TriggeredRunDeliveryRecord {
            run_id,
            outcome,
            recorded_at: Utc::now(),
        };
        self.delivery_store
            .record_triggered_run_delivery(record)
            .await
            .map_err(|reason| {
                PostSubmitDeliveryError::new(format!(
                    "Telegram terminal outcome persistence failed for run {run_id}: {reason}"
                ))
            })
    }
}

struct NoopTelegramConversationBindingService;

#[async_trait]
impl ConversationBindingService for NoopTelegramConversationBindingService {
    async fn resolve_binding(
        &self,
        _request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        Err(unsupported_triggered_binding())
    }

    async fn lookup_binding(
        &self,
        _request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        Err(unsupported_triggered_binding())
    }
}

fn unsupported_triggered_binding() -> ProductWorkflowError {
    ProductWorkflowError::BindingResolutionFailed {
        reason: "Telegram triggered delivery receives its turn scope from the trigger poller"
            .to_string(),
    }
}

fn invalid_config(field: &'static str, reason: String) -> TelegramHostBuildError {
    TelegramHostBuildError::InvalidConfig { field, reason }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn revision_cache_reuses_the_same_revision() {
        let cache = RevisionCache::default();
        let first = Arc::new("first");
        let installed = cache.install_unless_newer(7, Arc::clone(&first)).await;
        let repeated = cache.get(7).await.expect("revision is cached");

        assert!(Arc::ptr_eq(&installed, &repeated));
    }

    #[tokio::test]
    async fn revision_cache_replaces_with_a_newer_revision() {
        let cache = RevisionCache::default();
        let old = cache.install_unless_newer(7, Arc::new("old")).await;
        let new = cache.install_unless_newer(8, Arc::new("new")).await;

        assert!(!Arc::ptr_eq(&old, &new));
        assert_eq!(*cache.get(8).await.expect("new revision is cached"), "new");
        assert!(cache.get(7).await.is_none());
    }

    #[tokio::test]
    async fn stale_revision_cannot_replace_a_newer_driver() {
        let cache = RevisionCache::default();
        let newest = cache.install_unless_newer(8, Arc::new("new")).await;
        let stale = cache.install_unless_newer(7, Arc::new("stale")).await;

        assert!(Arc::ptr_eq(&newest, &stale));
        assert_eq!(*cache.get(8).await.expect("new revision remains"), "new");
    }
}
