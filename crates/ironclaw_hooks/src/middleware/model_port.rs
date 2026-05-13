//! Model-port middleware that fires `after_model` observer hooks after each
//! successful `stream_model` call.
//!
//! By design, observers only learn that a model exchange happened — they
//! never see the raw model output. The trust model documented in
//! `CLAUDE.md` is explicit that Installed/Trusted hooks must not receive
//! ambient model data; the `ObservedKind::AfterModel` signal is the entire
//! payload an observer sees here.
//!
//! Observers fail isolated: an observer panic / timeout / missing impl
//! does not affect the model call's return value. Errors from the inner
//! `LoopModelPort` are forwarded unchanged and short-circuit observation
//! (we don't fire `after_model` on a failed exchange — that lands on a
//! distinct error point in a follow-up slice).

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::TenantId;
use ironclaw_turns::run_profile::{
    AgentLoopHostError, LoopModelPort, LoopModelRequest, LoopModelResponse,
};

use crate::dispatch::HookDispatcher;
use crate::registry::HookPointSpec;

/// Wraps an inner `LoopModelPort`, forwards `stream_model` unchanged, and
/// dispatches `after_model` observer hooks once the inner call returns
/// successfully.
pub struct HookedLoopModelPort {
    inner: Arc<dyn LoopModelPort>,
    dispatcher: Arc<HookDispatcher>,
    tenant_id: TenantId,
}

impl HookedLoopModelPort {
    pub fn new(
        inner: Arc<dyn LoopModelPort>,
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
impl LoopModelPort for HookedLoopModelPort {
    async fn stream_model(
        &self,
        request: LoopModelRequest,
    ) -> Result<LoopModelResponse, AgentLoopHostError> {
        let response = self.inner.stream_model(request).await?;
        let observed = self
            .dispatcher
            .dispatch_observer_at(HookPointSpec::AfterModel, self.tenant_id.clone())
            .await;
        tracing::debug!(
            facts = observed.facts.len(),
            failures = observed.failures.len(),
            "after_model observer dispatch completed"
        );
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::ObserverHookImpl;
    use crate::identity::{HookId, HookVersion};
    use crate::kinds::observer::NoteCategory;
    use crate::ordering::HookPhase;
    use crate::points::ObserverHookContext;
    use crate::registry::{HookBinding, HookRegistry};
    use crate::sink::{ObserverHook, ObserverSink};
    use crate::trust::HookTrustClass;
    use async_trait::async_trait;
    use ironclaw_turns::run_profile::{
        AssistantReply, LoopModelRequest, LoopModelResponse, ModelProfileId, ParentLoopOutput,
    };
    use std::sync::Mutex;

    fn tenant() -> TenantId {
        TenantId::new("alpha").expect("ok")
    }

    struct StubModelPort {
        calls: Mutex<u32>,
        fail: bool,
    }

    impl StubModelPort {
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
    impl LoopModelPort for StubModelPort {
        async fn stream_model(
            &self,
            _request: LoopModelRequest,
        ) -> Result<LoopModelResponse, AgentLoopHostError> {
            *self.calls.lock().expect("not poisoned") += 1;
            if self.fail {
                return Err(AgentLoopHostError::new(
                    ironclaw_turns::run_profile::AgentLoopHostErrorKind::Unavailable,
                    "stub failure",
                ));
            }
            Ok(LoopModelResponse {
                chunks: Vec::new(),
                output: ParentLoopOutput::AssistantReply(AssistantReply {
                    content: "hi".to_string(),
                }),
                effective_model_profile_id: ModelProfileId::new("model_test").expect("ok"),
            })
        }
    }

    struct RecordingObserver {
        seen: Arc<Mutex<u32>>,
    }

    #[async_trait]
    impl ObserverHook for RecordingObserver {
        async fn observe(&self, _ctx: &ObserverHookContext, sink: &mut dyn ObserverSink) {
            *self.seen.lock().expect("not poisoned") += 1;
            sink.note(NoteCategory::HookFired, "after_model fired");
        }
    }

    struct PanickingObserver;

    #[async_trait]
    impl ObserverHook for PanickingObserver {
        async fn observe(&self, _ctx: &ObserverHookContext, _sink: &mut dyn ObserverSink) {
            panic!("intentional observer panic");
        }
    }

    fn request() -> LoopModelRequest {
        LoopModelRequest {
            messages: Vec::new(),
            surface_version: None,
            model_preference: None,
        }
    }

    fn observer_dispatcher_with(observer: ObserverHookImpl) -> Arc<HookDispatcher> {
        let id = HookId::for_builtin("test::after_model", HookVersion::ONE);
        let mut registry = HookRegistry::new();
        registry
            .insert(HookBinding {
                hook_id: id,
                hook_version: HookVersion::ONE,
                trust_class: HookTrustClass::Builtin,
                phase: HookPhase::Telemetry,
                point: HookPointSpec::AfterModel,
                owning_extension: None,
                scope: crate::registry::HookBindingScope::Global,
                poisoned: false,
            })
            .expect("ok");
        let mut dispatcher = HookDispatcher::new(registry);
        dispatcher.install_observer_impl(id, observer);
        Arc::new(dispatcher)
    }

    #[tokio::test]
    async fn forwards_to_inner_when_no_hooks() {
        let inner = Arc::new(StubModelPort::new());
        let dispatcher = Arc::new(HookDispatcher::new(HookRegistry::new()));
        let wrapped = HookedLoopModelPort::new(inner.clone(), dispatcher, tenant());

        wrapped.stream_model(request()).await.expect("ok");
        assert_eq!(inner.call_count(), 1);
    }

    #[tokio::test]
    async fn observer_fires_after_inner_call() {
        let inner = Arc::new(StubModelPort::new());
        let seen = Arc::new(Mutex::new(0u32));
        let dispatcher =
            observer_dispatcher_with(ObserverHookImpl::Any(Box::new(RecordingObserver {
                seen: seen.clone(),
            })));
        let wrapped = HookedLoopModelPort::new(inner.clone(), dispatcher, tenant());

        wrapped.stream_model(request()).await.expect("ok");
        assert_eq!(inner.call_count(), 1);
        assert_eq!(*seen.lock().expect("not poisoned"), 1);
    }

    #[tokio::test]
    async fn observer_failure_does_not_fail_outer_call() {
        let inner = Arc::new(StubModelPort::new());
        let dispatcher =
            observer_dispatcher_with(ObserverHookImpl::Any(Box::new(PanickingObserver)));
        let wrapped = HookedLoopModelPort::new(inner.clone(), dispatcher, tenant());

        let result = wrapped.stream_model(request()).await;
        assert!(
            result.is_ok(),
            "panicking observer must not fail the outer call"
        );
        assert_eq!(inner.call_count(), 1);
    }

    #[tokio::test]
    async fn inner_error_propagates_and_skips_observers() {
        let inner = Arc::new(StubModelPort::failing());
        let seen = Arc::new(Mutex::new(0u32));
        let dispatcher =
            observer_dispatcher_with(ObserverHookImpl::Any(Box::new(RecordingObserver {
                seen: seen.clone(),
            })));
        let wrapped = HookedLoopModelPort::new(inner.clone(), dispatcher, tenant());

        let err = wrapped.stream_model(request()).await.expect_err("must err");
        assert_eq!(
            err.kind,
            ironclaw_turns::run_profile::AgentLoopHostErrorKind::Unavailable
        );
        assert_eq!(*seen.lock().expect("not poisoned"), 0);
    }
}
