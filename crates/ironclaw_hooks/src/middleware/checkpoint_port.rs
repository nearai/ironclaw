//! Checkpoint-port middleware that fires `after_checkpoint` observer hooks
//! after each successful `checkpoint` call.
//!
//! Checkpoints are durable facts — observers only see that a checkpoint was
//! written, never its state contents. Observation runs after the inner port
//! returns success; errors from the inner port forward unchanged and skip
//! observer dispatch. `stage_checkpoint_payload` and `load_checkpoint_payload`
//! are transparent pass-throughs to `inner` — they carry redacted payload
//! bytes the executor stages/loads around every `checkpoint` call, not a
//! completion fact, so they aren't observation points.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::TenantId;
use ironclaw_turns::TurnCheckpointId;
use ironclaw_turns::run_profile::{
    AgentLoopHostError, LoadCheckpointPayloadRequest, LoadedCheckpointPayload, LoopCheckpointPort,
    LoopCheckpointRequest, LoopCheckpointStateRef, StageCheckpointPayloadRequest,
};

use crate::dispatch::HookDispatcher;
use crate::registry::HookPointSpec;

/// Wraps an inner `LoopCheckpointPort`. Only `checkpoint` (the durable
/// metadata write) is an interception point that dispatches `after_checkpoint`
/// observer hooks; every other method is a transparent pass-through to
/// `inner` so the wrapper never changes checkpoint availability.
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

    async fn stage_checkpoint_payload(
        &self,
        request: StageCheckpointPayloadRequest,
    ) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
        self.inner.stage_checkpoint_payload(request).await
    }

    async fn load_checkpoint_payload(
        &self,
        request: LoadCheckpointPayloadRequest,
    ) -> Result<LoadedCheckpointPayload, AgentLoopHostError> {
        self.inner.load_checkpoint_payload(request).await
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
        stage_calls: Mutex<u32>,
        load_calls: Mutex<u32>,
        fail: bool,
    }

    impl StubCheckpointPort {
        fn new() -> Self {
            Self {
                calls: Mutex::new(0),
                stage_calls: Mutex::new(0),
                load_calls: Mutex::new(0),
                fail: false,
            }
        }

        fn failing() -> Self {
            Self {
                calls: Mutex::new(0),
                stage_calls: Mutex::new(0),
                load_calls: Mutex::new(0),
                fail: true,
            }
        }

        fn call_count(&self) -> u32 {
            *self.calls.lock().expect("not poisoned")
        }

        fn stage_call_count(&self) -> u32 {
            *self.stage_calls.lock().expect("not poisoned")
        }

        fn load_call_count(&self) -> u32 {
            *self.load_calls.lock().expect("not poisoned")
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

        async fn stage_checkpoint_payload(
            &self,
            request: ironclaw_turns::run_profile::StageCheckpointPayloadRequest,
        ) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
            *self.stage_calls.lock().expect("not poisoned") += 1;
            let _ = request;
            if self.fail {
                return Err(AgentLoopHostError::new(
                    ironclaw_turns::run_profile::AgentLoopHostErrorKind::Invalid,
                    "stub stage failure",
                ));
            }
            LoopCheckpointStateRef::new("checkpoint:stub-staged").map_err(|reason| {
                AgentLoopHostError::new(
                    ironclaw_turns::run_profile::AgentLoopHostErrorKind::Internal,
                    reason,
                )
            })
        }

        async fn load_checkpoint_payload(
            &self,
            request: ironclaw_turns::run_profile::LoadCheckpointPayloadRequest,
        ) -> Result<ironclaw_turns::run_profile::LoadedCheckpointPayload, AgentLoopHostError>
        {
            *self.load_calls.lock().expect("not poisoned") += 1;
            if self.fail {
                return Err(AgentLoopHostError::new(
                    ironclaw_turns::run_profile::AgentLoopHostErrorKind::Invalid,
                    "stub load failure",
                ));
            }
            Ok(ironclaw_turns::run_profile::LoadedCheckpointPayload {
                kind: LoopCheckpointKind::BeforeModel,
                schema_id: request.expected_schema_id,
                schema_version: request.expected_schema_version,
                payload: ironclaw_turns::RedactedCheckpointPayload::new(Vec::new())
                    .expect("empty payload is within the size limit"),
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

    fn request() -> LoopCheckpointRequest {
        LoopCheckpointRequest {
            kind: LoopCheckpointKind::BeforeModel,
            state_ref: LoopCheckpointStateRef::new("checkpoint:test-0001").expect("ok"),
            gate_ref: None,
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

    /// A coordinator turn stages a checkpoint payload before writing the
    /// checkpoint itself; if the wrapper falls through to the trait's
    /// fail-closed default instead of forwarding to `inner`, every
    /// hooks-enabled turn dies here before any hook fires.
    #[tokio::test]
    async fn stage_and_load_checkpoint_payload_forward_to_inner() {
        let inner = Arc::new(StubCheckpointPort::new());
        let dispatcher = Arc::new(HookDispatcher::new(HookRegistry::new()));
        let wrapped = HookedLoopCheckpointPort::new(inner.clone(), dispatcher, tenant());

        wrapped
            .stage_checkpoint_payload(ironclaw_turns::run_profile::StageCheckpointPayloadRequest {
                kind: LoopCheckpointKind::BeforeModel,
                schema_id: "test-schema".to_string(),
                payload: b"payload".to_vec(),
            })
            .await
            .expect("forwarded stage call succeeds");
        assert_eq!(inner.stage_call_count(), 1);

        wrapped
            .load_checkpoint_payload(ironclaw_turns::run_profile::LoadCheckpointPayloadRequest {
                checkpoint_id: TurnCheckpointId::new(),
                expected_schema_id: ironclaw_turns::run_profile::CheckpointSchemaId::new(
                    "test-schema",
                )
                .expect("ok"),
                expected_schema_version: ironclaw_turns::RunProfileVersion::new(1),
            })
            .await
            .expect("forwarded load call succeeds");
        assert_eq!(inner.load_call_count(), 1);
    }

    /// Mirrors the happy-path forwarding test above but with a failing inner
    /// port: both pass-throughs must propagate the inner error unchanged
    /// rather than swallowing it or falling through to a default.
    #[tokio::test]
    async fn stage_and_load_checkpoint_payload_forward_inner_error() {
        let inner = Arc::new(StubCheckpointPort::failing());
        let dispatcher = Arc::new(HookDispatcher::new(HookRegistry::new()));
        let wrapped = HookedLoopCheckpointPort::new(inner.clone(), dispatcher, tenant());

        let stage_err = wrapped
            .stage_checkpoint_payload(ironclaw_turns::run_profile::StageCheckpointPayloadRequest {
                kind: LoopCheckpointKind::BeforeModel,
                schema_id: "test-schema".to_string(),
                payload: b"payload".to_vec(),
            })
            .await
            .expect_err("inner stage error must propagate");
        assert_eq!(
            stage_err.kind,
            ironclaw_turns::run_profile::AgentLoopHostErrorKind::Invalid
        );
        assert_eq!(inner.stage_call_count(), 1);

        let load_err = wrapped
            .load_checkpoint_payload(ironclaw_turns::run_profile::LoadCheckpointPayloadRequest {
                checkpoint_id: TurnCheckpointId::new(),
                expected_schema_id: ironclaw_turns::run_profile::CheckpointSchemaId::new(
                    "test-schema",
                )
                .expect("ok"),
                expected_schema_version: ironclaw_turns::RunProfileVersion::new(1),
            })
            .await
            .expect_err("inner load error must propagate");
        assert_eq!(
            load_err.kind,
            ironclaw_turns::run_profile::AgentLoopHostErrorKind::Invalid
        );
        assert_eq!(inner.load_call_count(), 1);
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
