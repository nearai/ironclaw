//! Model-port middleware: pass-through wrapper.
//!
//! **`AfterModel` does NOT fire here.** Earlier slices dispatched
//! `HookPointSpec::AfterModel` from `stream_model`, but
//! `HookedLoopTranscriptPort::finalize_assistant_message` also dispatches
//! the same point. That double-fire combined with the fact that the
//! model-port dispatch happened **before** the assistant reply was durable
//! could leave an observer event recorded for a reply that never finalized
//! (henrypark133 Concerning #5).
//!
//! Authoritative `AfterModel` boundary: the transcript port (after
//! `finalize_assistant_message` succeeds). The wrapper here is preserved
//! as a no-op forwarding shim so the factory's wrap-every-port pattern
//! stays symmetric and so we have a hook point for a future
//! `model-response-observed` (pre-durable) signal if a use case justifies
//! it.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::TenantId;
use ironclaw_turns::run_profile::{
    AgentLoopHostError, LoopModelPort, LoopModelRequest, LoopModelResponse,
};

use crate::dispatch::HookDispatcher;

/// Wraps an inner `LoopModelPort`, forwards `stream_model` unchanged, and
/// dispatches `after_model` observer hooks once the inner call returns
/// successfully.
pub struct HookedLoopModelPort {
    inner: Arc<dyn LoopModelPort>,
    /// Kept for future point-specific observers (e.g., `model-response-
    /// observed` at the pre-durable boundary). Currently unused — the
    /// model port is a no-op wrapper.
    #[allow(dead_code)]
    dispatcher: Arc<HookDispatcher>,
    #[allow(dead_code)]
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
        // No-op wrapper. AfterModel observers fire from the transcript
        // port at the durable-finalization boundary, not here. See module
        // docs.
        self.inner.stream_model(request).await
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
    use crate::registry::HookPointSpec;
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
            capability_view: None,
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
                priority: HookPriority::DEFAULT,
                point: HookPointSpec::AfterModel,
                event_kind_filter: None,
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

    /// After Concerning #5 (AfterModel exactly-once), the model port no
    /// longer fires AfterModel — the transcript port owns that boundary.
    /// This test pins the new behavior: an AfterModel observer wired
    /// against a model-port wrapper sees nothing.
    #[tokio::test]
    async fn model_port_does_not_fire_after_model_observers() {
        let inner = Arc::new(StubModelPort::new());
        let seen = Arc::new(Mutex::new(0u32));
        let dispatcher =
            observer_dispatcher_with(ObserverHookImpl::Any(Box::new(RecordingObserver {
                seen: seen.clone(),
            })));
        let wrapped = HookedLoopModelPort::new(inner.clone(), dispatcher, tenant());

        wrapped.stream_model(request()).await.expect("ok");
        assert_eq!(inner.call_count(), 1);
        assert_eq!(
            *seen.lock().expect("not poisoned"),
            0,
            "AfterModel must NOT fire from the model port — the transcript port owns it"
        );
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
