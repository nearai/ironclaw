//! Late-bound runtime state for the web-push channel.
//!
//! The channel adapter is constructed by the binary's binding table before
//! composition has built storage, so the adapter holds this slot and
//! composition installs the runtime exactly once at assembly (the same boot
//! ordering the trigger poller's buffered post-submit hook resolves). Until
//! installed, consumers fail closed with `WebPushError::RuntimeUnavailable`.

use std::sync::{Arc, RwLock};

use crate::error::WebPushError;
use crate::store::WebPushSubscriptionStore;

/// Everything the delivery adapter needs at send time.
pub struct WebPushRuntime {
    pub subscriptions: Arc<dyn WebPushSubscriptionStore>,
}

/// Cloneable installer/consumer handle around the runtime.
#[derive(Clone, Default)]
pub struct WebPushRuntimeSlot {
    inner: Arc<RwLock<Option<Arc<WebPushRuntime>>>>,
}

impl WebPushRuntimeSlot {
    pub fn new() -> Self {
        Self::default()
    }

    /// Install the runtime. Exactly once per process; a second install is a
    /// wiring bug and fails loudly.
    pub fn install(&self, runtime: Arc<WebPushRuntime>) -> Result<(), WebPushError> {
        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.is_some() {
            return Err(WebPushError::RuntimeAlreadyInstalled);
        }
        *guard = Some(runtime);
        Ok(())
    }

    pub fn get(&self) -> Result<Arc<WebPushRuntime>, WebPushError> {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .ok_or(WebPushError::RuntimeUnavailable)
    }

    pub fn is_installed(&self) -> bool {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{PushSubscriptionUpsertOutcome, WebPushSubscriptionStore};
    use crate::subscription::{PushEndpoint, PushSubscriptionRecord};
    use async_trait::async_trait;
    use ironclaw_host_api::resource::ResourceScope;

    struct NullStore;

    #[async_trait]
    impl WebPushSubscriptionStore for NullStore {
        async fn upsert_subscription(
            &self,
            _scope: &ResourceScope,
            _record: PushSubscriptionRecord,
        ) -> Result<PushSubscriptionUpsertOutcome, WebPushError> {
            Ok(PushSubscriptionUpsertOutcome::Enrolled)
        }

        async fn remove_subscription(
            &self,
            _scope: &ResourceScope,
            _endpoint: &PushEndpoint,
        ) -> Result<bool, WebPushError> {
            Ok(false)
        }

        async fn list_subscriptions(
            &self,
            _scope: &ResourceScope,
        ) -> Result<Vec<PushSubscriptionRecord>, WebPushError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn slot_fails_closed_then_installs_exactly_once() {
        let slot = WebPushRuntimeSlot::new();
        assert!(matches!(slot.get(), Err(WebPushError::RuntimeUnavailable)));
        assert!(!slot.is_installed());

        let runtime = Arc::new(WebPushRuntime {
            subscriptions: Arc::new(NullStore),
        });
        slot.install(Arc::clone(&runtime)).expect("first install");
        assert!(slot.is_installed());
        assert!(slot.get().is_ok());
        assert!(matches!(
            slot.install(runtime),
            Err(WebPushError::RuntimeAlreadyInstalled)
        ));
    }
}
