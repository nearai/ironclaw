use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_triggers::TriggerFire;
use ironclaw_turns::{TurnRunId, TurnScope};

/// Hook invoked by the trigger poller after a successful fire is durably
/// settled. Implementations own channel-neutral post-submit delivery behavior.
#[async_trait]
pub trait PostSubmitDeliveryHook: Send + Sync {
    /// Called with the original trigger fire, the submitted run id, and the
    /// turn scope the run was submitted under. The trigger poller invokes this
    /// hook from a detached task after the accepted fire appears as settled, so
    /// hook latency cannot delay settlement and delivery cannot precede the
    /// persisted run/thread mapping. Implementations may still spawn their own
    /// longer-lived delivery tasks when they need bounded admission or shutdown
    /// tracking.
    async fn on_trigger_submitted(&self, fire: TriggerFire, run_id: TurnRunId, scope: TurnScope);
}

/// No-op hook used when the Slack host-beta feature is not active.
pub struct NoopPostSubmitDeliveryHook;

#[async_trait]
impl PostSubmitDeliveryHook for NoopPostSubmitDeliveryHook {
    async fn on_trigger_submitted(
        &self,
        _fire: TriggerFire,
        _run_id: TurnRunId,
        _scope: TurnScope,
    ) {
    }
}

/// Key-deduplicated fan-out over multiple [`PostSubmitDeliveryHook`]s.
///
/// The trigger poller's post-submit slot is a single `OnceLock`; with more
/// than one channel host (Slack + Telegram) each needs its own hook. The
/// runtime installs one composite into that slot on the first
/// `add_trigger_post_submit_hook` call and appends later hooks to it, so the
/// The poller-side consumer is unchanged.
///
/// Keys are per-host constants (e.g. `slack-host-beta`): a second add under an
/// existing key is rejected (`false`) instead of appended, preserving the old
/// single-slot idempotency — a host that mounts twice never double-delivers.
#[derive(Default)]
pub struct CompositePostSubmitDeliveryHook {
    hooks: std::sync::RwLock<Vec<(String, Arc<dyn PostSubmitDeliveryHook>)>>,
}

impl CompositePostSubmitDeliveryHook {
    /// Append `hook` under `hook_key`. Returns `false` (and drops the hook)
    /// when the key is already registered.
    pub fn add(&self, hook_key: &str, hook: Arc<dyn PostSubmitDeliveryHook>) -> bool {
        let mut hooks = self
            .hooks
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if hooks.iter().any(|(existing, _)| existing == hook_key) {
            return false;
        }
        hooks.push((hook_key.to_string(), hook));
        true
    }

    fn snapshot(&self) -> Vec<Arc<dyn PostSubmitDeliveryHook>> {
        self.hooks
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|(_, hook)| Arc::clone(hook))
            .collect()
    }
}

impl std::fmt::Debug for CompositePostSubmitDeliveryHook {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let keys: Vec<String> = self
            .hooks
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|(key, _)| key.clone())
            .collect();
        formatter
            .debug_struct("CompositePostSubmitDeliveryHook")
            .field("hooks", &keys)
            .finish()
    }
}

#[async_trait]
impl PostSubmitDeliveryHook for CompositePostSubmitDeliveryHook {
    async fn on_trigger_submitted(&self, fire: TriggerFire, run_id: TurnRunId, scope: TurnScope) {
        // Each hook runs in its own task so one slow or panicking hook cannot
        // delay or skip the others; the poller already detaches the whole
        // settlement (`spawn_post_submit_delivery`), so awaiting the joins here
        // only bounds this composite call, not trigger settlement.
        let handles: Vec<tokio::task::JoinHandle<()>> = self
            .snapshot()
            .into_iter()
            .map(|hook| {
                let fire = fire.clone();
                let scope = scope.clone();
                tokio::spawn(async move { hook.on_trigger_submitted(fire, run_id, scope).await })
            })
            .collect();
        for handle in handles {
            if let Err(error) = handle.await {
                tracing::debug!(
                    target = "ironclaw::reborn::trigger_delivery",
                    %run_id,
                    %error,
                    "post-submit delivery hook task failed; other hooks were unaffected"
                );
            }
        }
    }
}
