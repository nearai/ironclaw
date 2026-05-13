//! Transcript-port middleware that observes transcript finalization.
//!
//! The `LoopTranscriptPort` trait has four methods (begin / update / finalize
//! assistant draft, plus append capability result ref). Only
//! `finalize_assistant_message` is observed: it's the natural "model exchange
//! durable" boundary — the point at which an assistant turn becomes a fact
//! the rest of the system can act on. Drafts (begin / update) are transient
//! and `append_capability_result_ref` already has a dedicated
//! `after_capability` observation point fired by the capability-port
//! middleware.
//!
//! Observers fire `HookPointSpec::AfterModel` here (the same point fired by
//! `HookedLoopModelPort`). They learn only that a finalization happened, never
//! the message content. Errors from the inner port are forwarded unchanged
//! and short-circuit observation.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::TenantId;
use ironclaw_turns::LoopMessageRef;
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AppendCapabilityResultRef, BeginAssistantDraft, FinalizeAssistantMessage,
    LoopTranscriptPort, UpdateAssistantDraft,
};

use crate::dispatch::HookDispatcher;
use crate::registry::HookPointSpec;

/// Wraps an inner `LoopTranscriptPort`. After a successful
/// `finalize_assistant_message` call, dispatches `after_model` observer
/// hooks. All other methods are forwarded unchanged.
pub struct HookedLoopTranscriptPort {
    inner: Arc<dyn LoopTranscriptPort>,
    dispatcher: Arc<HookDispatcher>,
    tenant_id: TenantId,
}

impl HookedLoopTranscriptPort {
    pub fn new(
        inner: Arc<dyn LoopTranscriptPort>,
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
impl LoopTranscriptPort for HookedLoopTranscriptPort {
    async fn begin_assistant_draft(
        &self,
        request: BeginAssistantDraft,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        self.inner.begin_assistant_draft(request).await
    }

    async fn update_assistant_draft(
        &self,
        request: UpdateAssistantDraft,
    ) -> Result<(), AgentLoopHostError> {
        self.inner.update_assistant_draft(request).await
    }

    async fn finalize_assistant_message(
        &self,
        request: FinalizeAssistantMessage,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        let message_ref = self.inner.finalize_assistant_message(request).await?;
        let observed = self
            .dispatcher
            .dispatch_observer_at(HookPointSpec::AfterModel, self.tenant_id.clone())
            .await;
        tracing::debug!(
            facts = observed.facts.len(),
            failures = observed.failures.len(),
            "after_model observer dispatch completed for transcript finalize"
        );
        Ok(message_ref)
    }

    async fn append_capability_result_ref(
        &self,
        request: AppendCapabilityResultRef,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        self.inner.append_capability_result_ref(request).await
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
    use ironclaw_turns::run_profile::AssistantReply;
    use std::sync::Mutex;

    fn tenant() -> TenantId {
        TenantId::new("alpha").expect("ok")
    }

    fn message_ref() -> LoopMessageRef {
        LoopMessageRef::new("msg:test-finalize-0001").expect("ok")
    }

    struct StubTranscriptPort {
        finalize_calls: Mutex<u32>,
        fail: bool,
    }

    impl StubTranscriptPort {
        fn new() -> Self {
            Self {
                finalize_calls: Mutex::new(0),
                fail: false,
            }
        }

        fn failing() -> Self {
            Self {
                finalize_calls: Mutex::new(0),
                fail: true,
            }
        }

        fn finalize_call_count(&self) -> u32 {
            *self.finalize_calls.lock().expect("not poisoned")
        }
    }

    #[async_trait]
    impl LoopTranscriptPort for StubTranscriptPort {
        async fn finalize_assistant_message(
            &self,
            _request: FinalizeAssistantMessage,
        ) -> Result<LoopMessageRef, AgentLoopHostError> {
            *self.finalize_calls.lock().expect("not poisoned") += 1;
            if self.fail {
                return Err(AgentLoopHostError::new(
                    ironclaw_turns::run_profile::AgentLoopHostErrorKind::TranscriptWriteFailed,
                    "stub finalize failure",
                ));
            }
            Ok(message_ref())
        }
    }

    struct RecordingObserver {
        seen: Arc<Mutex<u32>>,
    }

    #[async_trait]
    impl ObserverHook for RecordingObserver {
        async fn observe(&self, _ctx: &ObserverHookContext, sink: &mut dyn ObserverSink) {
            *self.seen.lock().expect("not poisoned") += 1;
            sink.note(NoteCategory::HookFired, "after_finalize");
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
        let id = HookId::for_builtin("test::after_model_transcript", HookVersion::ONE);
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

    fn request() -> FinalizeAssistantMessage {
        FinalizeAssistantMessage {
            reply: AssistantReply {
                content: "done".to_string(),
            },
        }
    }

    #[tokio::test]
    async fn forwards_to_inner_when_no_hooks() {
        let inner = Arc::new(StubTranscriptPort::new());
        let dispatcher = Arc::new(HookDispatcher::new(HookRegistry::new()));
        let wrapped = HookedLoopTranscriptPort::new(inner.clone(), dispatcher, tenant());

        wrapped
            .finalize_assistant_message(request())
            .await
            .expect("ok");
        assert_eq!(inner.finalize_call_count(), 1);
    }

    #[tokio::test]
    async fn observer_fires_after_finalize() {
        let inner = Arc::new(StubTranscriptPort::new());
        let seen = Arc::new(Mutex::new(0u32));
        let dispatcher =
            observer_dispatcher_with(ObserverHookImpl::Any(Box::new(RecordingObserver {
                seen: seen.clone(),
            })));
        let wrapped = HookedLoopTranscriptPort::new(inner.clone(), dispatcher, tenant());

        wrapped
            .finalize_assistant_message(request())
            .await
            .expect("ok");
        assert_eq!(inner.finalize_call_count(), 1);
        assert_eq!(*seen.lock().expect("not poisoned"), 1);
    }

    #[tokio::test]
    async fn observer_failure_does_not_fail_outer_call() {
        let inner = Arc::new(StubTranscriptPort::new());
        let dispatcher =
            observer_dispatcher_with(ObserverHookImpl::Any(Box::new(PanickingObserver)));
        let wrapped = HookedLoopTranscriptPort::new(inner.clone(), dispatcher, tenant());

        let result = wrapped.finalize_assistant_message(request()).await;
        assert!(
            result.is_ok(),
            "panicking observer must not fail the outer call"
        );
        assert_eq!(inner.finalize_call_count(), 1);
    }

    #[tokio::test]
    async fn inner_error_propagates_and_skips_observers() {
        let inner = Arc::new(StubTranscriptPort::failing());
        let seen = Arc::new(Mutex::new(0u32));
        let dispatcher =
            observer_dispatcher_with(ObserverHookImpl::Any(Box::new(RecordingObserver {
                seen: seen.clone(),
            })));
        let wrapped = HookedLoopTranscriptPort::new(inner.clone(), dispatcher, tenant());

        let err = wrapped
            .finalize_assistant_message(request())
            .await
            .expect_err("must err");
        assert_eq!(
            err.kind,
            ironclaw_turns::run_profile::AgentLoopHostErrorKind::TranscriptWriteFailed
        );
        assert_eq!(*seen.lock().expect("not poisoned"), 0);
    }
}
