//! Checkpoint-port middleware that fires `after_checkpoint` observer hooks
//! after each successful `checkpoint` call.
//!
//! Checkpoints are durable facts — observers only see that a checkpoint was
//! written, never its state contents. Observation runs after the inner port
//! returns success; errors from the inner port forward unchanged and skip
//! observer dispatch.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::TenantId;
use ironclaw_turns::TurnCheckpointId;
use ironclaw_turns::run_profile::{AgentLoopHostError, LoopCheckpointPort, LoopCheckpointRequest};

use crate::dispatch::HookDispatcher;
use crate::registry::HookPointSpec;

/// Wraps an inner `LoopCheckpointPort`, forwards `checkpoint` unchanged, and
/// dispatches `after_checkpoint` observer hooks after a successful write.
pub struct HookedLoopCheckpointPort {
    inner: Arc<dyn LoopCheckpointPort>,
    dispatcher: Arc<HookDispatcher>,
    tenant_id: TenantId,
}

impl HookedLoopCheckpointPort {
    pub fn new(
        inner: Arc<dyn LoopCheckpointPort>,
        dispatcher: Arc<HookDispatcher>,
        tenant_id: TenantId,
    ) -> Self {
        Self {
            inner,
            dispatcher,
            tenant_id,
        }
    }
}

#[async_trait]
impl LoopCheckpointPort for HookedLoopCheckpointPort {
    async fn checkpoint(
        &self,
        request: LoopCheckpointRequest,
    ) -> Result<TurnCheckpointId, AgentLoopHostError> {
        let checkpoint_id = self.inner.checkpoint(request).await?;
        let observed = self
            .dispatcher
            .dispatch_observer_at(HookPointSpec::AfterCheckpoint, self.tenant_id.clone())
            .await;
        tracing::debug!(
            facts = observed.facts.len(),
            failures = observed.failures.len(),
            "after_checkpoint observer dispatch completed"
        );
        Ok(checkpoint_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::ObserverHookImpl;
    use crate::identity::{HookId, HookVersion};
    use crate::kinds::observer::NoteCategory;
    use crate::ordering::HookPhase;
    use crate::ordering::HookPriority;
    use crate::points::ObserverHookContext;
    use crate::registry::{HookBinding, HookRegistry};
    use crate::sink::{ObserverHook, ObserverSink};
    use crate::trust::HookTrustClass;
    use async_trait::async_trait;
    use ironclaw_turns::run_profile::{LoopCheckpointKind, LoopCheckpointStateRef};
    use std::sync::Mutex;

    fn tenant() -> TenantId {
        TenantId::new("alpha").expect("ok")
    }

    struct StubCheckpointPort {
        calls: Mutex<u32>,
        fail: bool,
    }

    impl StubCheckpointPort {
        fn new() -> Self {
            Self {
                calls: Mutex::new(0),
                fail: false,
            }
        }

        fn failing() -> Self {
            Self {
                calls: Mutex::new(0),
                fail: true,
            }
        }

        fn call_count(&self) -> u32 {
            *self.calls.lock().expect("not poisoned")
        }
    }

    #[async_trait]
    impl LoopCheckpointPort for StubCheckpointPort {
        async fn checkpoint(
            &self,
            _request: LoopCheckpointRequest,
        ) -> Result<TurnCheckpointId, AgentLoopHostError> {
            *self.calls.lock().expect("not poisoned") += 1;
            if self.fail {
                return Err(AgentLoopHostError::new(
                    ironclaw_turns::run_profile::AgentLoopHostErrorKind::CheckpointRejected,
                    "stub checkpoint failure",
                ));
            }
            Ok(TurnCheckpointId::new())
        }
    }

    struct RecordingObserver {
        seen: Arc<Mutex<u32>>,
    }

    #[async_trait]
    impl ObserverHook for RecordingObserver {
        async fn observe(&self, _ctx: &ObserverHookContext, sink: &mut dyn ObserverSink) {
            *self.seen.lock().expect("not poisoned") += 1;
            sink.note(NoteCategory::HookFired, "after_checkpoint fired");
        }
    }

    struct PanickingObserver;

    #[async_trait]
    impl ObserverHook for PanickingObserver {
        async fn observe(&self, _ctx: &ObserverHookContext, _sink: &mut dyn ObserverSink) {
            panic!("intentional observer panic");
        }
    }

    fn observer_dispatcher_with(observer: ObserverHookImpl) -> Arc<HookDispatcher> {
        let id = HookId::for_builtin("test::after_checkpoint", HookVersion::ONE);
        let mut registry = HookRegistry::new();
        registry
            .insert(HookBinding {
                hook_id: id,
                hook_version: HookVersion::ONE,
                trust_class: HookTrustClass::Builtin,
                phase: HookPhase::Telemetry,
                priority: HookPriority::DEFAULT,
                point: HookPointSpec::AfterCheckpoint,
                owning_extension: None,
                scope: crate::registry::HookBindingScope::Global,
                poisoned: false,
            })
            .expect("ok");
        let mut dispatcher = HookDispatcher::new(registry);
        dispatcher.install_observer_impl(id, observer);
        Arc::new(dispatcher)
    }

    fn request() -> LoopCheckpointRequest {
        LoopCheckpointRequest {
            kind: LoopCheckpointKind::BeforeModel,
            state_ref: LoopCheckpointStateRef::new("checkpoint:test-0001").expect("ok"),
        }
    }

    #[tokio::test]
    async fn forwards_to_inner_when_no_hooks() {
        let inner = Arc::new(StubCheckpointPort::new());
        let dispatcher = Arc::new(HookDispatcher::new(HookRegistry::new()));
        let wrapped = HookedLoopCheckpointPort::new(inner.clone(), dispatcher, tenant());

        wrapped.checkpoint(request()).await.expect("ok");
        assert_eq!(inner.call_count(), 1);
    }

    #[tokio::test]
    async fn observer_fires_after_inner_call() {
        let inner = Arc::new(StubCheckpointPort::new());
        let seen = Arc::new(Mutex::new(0u32));
        let dispatcher =
            observer_dispatcher_with(ObserverHookImpl::Any(Box::new(RecordingObserver {
                seen: seen.clone(),
            })));
        let wrapped = HookedLoopCheckpointPort::new(inner.clone(), dispatcher, tenant());

        wrapped.checkpoint(request()).await.expect("ok");
        assert_eq!(inner.call_count(), 1);
        assert_eq!(*seen.lock().expect("not poisoned"), 1);
    }

    #[tokio::test]
    async fn observer_failure_does_not_fail_outer_call() {
        let inner = Arc::new(StubCheckpointPort::new());
        let dispatcher =
            observer_dispatcher_with(ObserverHookImpl::Any(Box::new(PanickingObserver)));
        let wrapped = HookedLoopCheckpointPort::new(inner.clone(), dispatcher, tenant());

        let result = wrapped.checkpoint(request()).await;
        assert!(
            result.is_ok(),
            "panicking observer must not fail the outer call"
        );
        assert_eq!(inner.call_count(), 1);
    }

    #[tokio::test]
    async fn inner_error_propagates_and_skips_observers() {
        let inner = Arc::new(StubCheckpointPort::failing());
        let seen = Arc::new(Mutex::new(0u32));
        let dispatcher =
            observer_dispatcher_with(ObserverHookImpl::Any(Box::new(RecordingObserver {
                seen: seen.clone(),
            })));
        let wrapped = HookedLoopCheckpointPort::new(inner.clone(), dispatcher, tenant());

        let err = wrapped.checkpoint(request()).await.expect_err("must err");
        assert_eq!(
            err.kind,
            ironclaw_turns::run_profile::AgentLoopHostErrorKind::CheckpointRejected
        );
        assert_eq!(*seen.lock().expect("not poisoned"), 0);
    }
}
