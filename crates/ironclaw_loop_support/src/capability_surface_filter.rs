use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityBatchInvocation, CapabilityBatchOutcome,
    CapabilityDenied, CapabilityDeniedReasonKind, CapabilityInvocation, CapabilityOutcome,
    LoopCapabilityPort, VisibleCapabilityRequest, VisibleCapabilitySurface,
};

use crate::CapabilityAllowSet;

#[derive(Clone)]
pub struct CapabilitySurfaceProfileFilter {
    inner: Arc<dyn LoopCapabilityPort>,
    allow_set: Arc<CapabilityAllowSet>,
}

impl CapabilitySurfaceProfileFilter {
    pub fn new(inner: Arc<dyn LoopCapabilityPort>, allow_set: Arc<CapabilityAllowSet>) -> Self {
        Self { inner, allow_set }
    }
}

#[async_trait]
impl LoopCapabilityPort for CapabilitySurfaceProfileFilter {
    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        let mut surface = self.inner.visible_capabilities(request).await?;
        if matches!(self.allow_set.as_ref(), CapabilityAllowSet::Allowlist(_)) {
            surface
                .descriptors
                .retain(|descriptor| self.allow_set.permits(&descriptor.capability_id));
        }
        Ok(surface)
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        if !self.allow_set.permits(&request.capability_id) {
            return Ok(surface_profile_denied_outcome());
        }
        self.inner.invoke_capability(request).await
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        if matches!(self.allow_set.as_ref(), CapabilityAllowSet::All) {
            return self.inner.invoke_capability_batch(request).await;
        }

        let mut slots = Vec::with_capacity(request.invocations.len());
        let mut allowed = Vec::new();
        let mut allowed_idx = Vec::new();

        for (index, invocation) in request.invocations.iter().enumerate() {
            if self.allow_set.permits(&invocation.capability_id) {
                allowed.push(invocation.clone());
                allowed_idx.push(index);
                slots.push(None);
            } else {
                slots.push(Some(surface_profile_denied_outcome()));
            }
        }

        let (inner_outcomes, stopped_on_suspension) = if allowed.is_empty() {
            (Vec::new(), false)
        } else {
            let inner_batch = self
                .inner
                .invoke_capability_batch(CapabilityBatchInvocation {
                    invocations: allowed,
                    stop_on_first_suspension: request.stop_on_first_suspension,
                })
                .await?;
            (inner_batch.outcomes, inner_batch.stopped_on_suspension)
        };

        if inner_outcomes.len() > allowed_idx.len() {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "capability surface filter received too many inner outcomes",
            ));
        }

        let n_inner = inner_outcomes.len();
        for (outcome, original_index) in inner_outcomes
            .into_iter()
            .zip(allowed_idx.iter().copied().take(n_inner))
        {
            slots[original_index] = Some(outcome);
        }

        // Truncate to the slot position after the last returned allowed outcome,
        // preserving interleaved denials up to that point.
        let truncate_to = if stopped_on_suspension && n_inner > 0 {
            allowed_idx[n_inner - 1] + 1
        } else if n_inner == allowed_idx.len() {
            slots.len()
        } else if n_inner == 0 {
            allowed_idx.first().copied().unwrap_or(slots.len())
        } else {
            allowed_idx[n_inner - 1] + 1
        };
        slots.truncate(truncate_to);

        let mut outcomes = Vec::with_capacity(slots.len());
        for slot in slots {
            let outcome = slot.ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "capability surface filter retained an unpopulated outcome slot",
                )
            })?;
            outcomes.push(outcome);
        }

        Ok(CapabilityBatchOutcome {
            outcomes,
            stopped_on_suspension,
        })
    }
}

fn surface_profile_denied_outcome() -> CapabilityOutcome {
    CapabilityOutcome::Denied(CapabilityDenied {
        reason_kind: surface_profile_denied_kind(),
        safe_summary: "capability not in run-profile surface".to_string(),
    })
}

fn surface_profile_denied_kind() -> CapabilityDeniedReasonKind {
    match CapabilityDeniedReasonKind::unknown("surface_profile_denied") {
        Ok(kind) => kind,
        Err(_) => {
            debug_assert!(
                false,
                "surface_profile_denied_kind() fallback reached — this is a contract bug: \
                 'surface_profile_denied' must be a valid reason kind value"
            );
            CapabilityDeniedReasonKind::EmptySurface
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use ironclaw_host_api::{CapabilityId, RuntimeKind};
    use ironclaw_turns::run_profile::{
        CapabilityDescriptorView, CapabilityInputRef, CapabilityResultMessage,
        CapabilitySurfaceVersion, ConcurrencyHint,
    };
    use ironclaw_turns::{LoopGateRef, LoopResultRef};

    use super::*;

    #[derive(Default)]
    struct SpyPort {
        surface: Mutex<Option<VisibleCapabilitySurface>>,
        batch_outcome: Mutex<Option<CapabilityBatchOutcome>>,
        visible_calls: Mutex<usize>,
        invocations: Mutex<Vec<CapabilityInvocation>>,
        batches: Mutex<Vec<CapabilityBatchInvocation>>,
    }

    #[async_trait]
    impl LoopCapabilityPort for SpyPort {
        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
            *self.visible_calls.lock().expect("visible calls lock") += 1;
            Ok(self
                .surface
                .lock()
                .expect("surface lock")
                .clone()
                .expect("test surface is configured"))
        }

        async fn invoke_capability(
            &self,
            request: CapabilityInvocation,
        ) -> Result<CapabilityOutcome, AgentLoopHostError> {
            self.invocations
                .lock()
                .expect("invocation lock")
                .push(request);
            Ok(completed("result:single"))
        }

        async fn invoke_capability_batch(
            &self,
            request: CapabilityBatchInvocation,
        ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
            self.batches.lock().expect("batch lock").push(request);
            Ok(self
                .batch_outcome
                .lock()
                .expect("batch outcome lock")
                .clone()
                .unwrap_or_else(|| CapabilityBatchOutcome {
                    outcomes: vec![completed("result:first"), completed("result:second")],
                    stopped_on_suspension: false,
                }))
        }
    }

    fn capability_id(value: &str) -> CapabilityId {
        CapabilityId::new(value).expect("test capability id is valid")
    }

    fn surface_version() -> CapabilitySurfaceVersion {
        CapabilitySurfaceVersion::new("surface-v1").expect("test version is valid")
    }

    fn input_ref(value: &str) -> CapabilityInputRef {
        CapabilityInputRef::new(value).expect("test input ref is valid")
    }

    fn invocation(capability: &str, input: &str) -> CapabilityInvocation {
        CapabilityInvocation {
            surface_version: surface_version(),
            capability_id: capability_id(capability),
            input_ref: input_ref(input),
        }
    }

    fn descriptor(capability: &str) -> CapabilityDescriptorView {
        CapabilityDescriptorView {
            capability_id: capability_id(capability),
            provider: None,
            runtime: RuntimeKind::Wasm,
            safe_name: capability.to_string(),
            safe_description: format!("{capability} description"),
            concurrency_hint: ConcurrencyHint::SafeForParallel,
        }
    }

    fn completed(result_ref: &str) -> CapabilityOutcome {
        CapabilityOutcome::Completed(CapabilityResultMessage {
            result_ref: LoopResultRef::new(result_ref).expect("test result ref is valid"),
            safe_summary: "done".to_string(),
            terminate_hint: false,
        })
    }

    fn approval_required(gate_ref: &str) -> CapabilityOutcome {
        CapabilityOutcome::ApprovalRequired {
            gate_ref: LoopGateRef::new(gate_ref).expect("test gate ref is valid"),
            safe_summary: "approval needed".to_string(),
        }
    }

    fn denied_reason(outcome: &CapabilityOutcome) -> Option<&str> {
        match outcome {
            CapabilityOutcome::Denied(denied) => Some(denied.reason_kind.as_str()),
            _ => None,
        }
    }

    #[tokio::test]
    async fn visible_capabilities_filters_descriptors() {
        let inner = Arc::new(SpyPort::default());
        *inner.surface.lock().expect("surface lock") = Some(VisibleCapabilitySurface {
            version: surface_version(),
            descriptors: vec![
                descriptor("demo.a"),
                descriptor("demo.b"),
                descriptor("demo.c"),
                descriptor("demo.d"),
                descriptor("demo.e"),
            ],
        });
        let filter = CapabilitySurfaceProfileFilter::new(
            inner,
            Arc::new(CapabilityAllowSet::allowlist([
                capability_id("demo.b"),
                capability_id("demo.d"),
            ])),
        );

        let surface = filter
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("surface");

        assert_eq!(surface.version, surface_version());
        assert_eq!(
            surface
                .descriptors
                .iter()
                .map(|descriptor| descriptor.capability_id.as_str())
                .collect::<Vec<_>>(),
            vec!["demo.b", "demo.d"]
        );
    }

    #[tokio::test]
    async fn invoke_denied_when_not_in_allowlist() {
        let inner = Arc::new(SpyPort::default());
        let filter = CapabilitySurfaceProfileFilter::new(
            inner.clone(),
            Arc::new(CapabilityAllowSet::allowlist([capability_id(
                "demo.allowed",
            )])),
        );

        let outcome = filter
            .invoke_capability(invocation("demo.denied", "input:denied"))
            .await
            .expect("outcome");

        assert_eq!(denied_reason(&outcome), Some("surface_profile_denied"));
        assert!(
            inner
                .invocations
                .lock()
                .expect("invocation lock")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn batch_partitions_correctly() {
        let inner = Arc::new(SpyPort::default());
        *inner.batch_outcome.lock().expect("batch outcome lock") = Some(CapabilityBatchOutcome {
            outcomes: vec![completed("result:first"), completed("result:second")],
            stopped_on_suspension: false,
        });
        let filter = CapabilitySurfaceProfileFilter::new(
            inner.clone(),
            Arc::new(CapabilityAllowSet::allowlist([
                capability_id("demo.first"),
                capability_id("demo.second"),
            ])),
        );

        let outcome = filter
            .invoke_capability_batch(CapabilityBatchInvocation {
                invocations: vec![
                    invocation("demo.first", "input:first"),
                    invocation("demo.denied", "input:denied"),
                    invocation("demo.second", "input:second"),
                ],
                stop_on_first_suspension: true,
            })
            .await
            .expect("batch outcome");

        assert_eq!(outcome.outcomes.len(), 3);
        assert_eq!(
            denied_reason(&outcome.outcomes[1]),
            Some("surface_profile_denied")
        );
        let batches = inner.batches.lock().expect("batch lock");
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].invocations.len(), 2);
        assert_eq!(
            batches[0].invocations[0].capability_id.as_str(),
            "demo.first"
        );
        assert_eq!(
            batches[0].invocations[1].capability_id.as_str(),
            "demo.second"
        );
    }

    #[tokio::test]
    async fn partial_inner_outcomes_truncate_correctly() {
        let inner = Arc::new(SpyPort::default());
        *inner.batch_outcome.lock().expect("batch outcome lock") = Some(CapabilityBatchOutcome {
            outcomes: vec![completed("result:first"), completed("result:second")],
            stopped_on_suspension: true,
        });
        let filter = CapabilitySurfaceProfileFilter::new(
            inner,
            Arc::new(CapabilityAllowSet::allowlist([
                capability_id("demo.first"),
                capability_id("demo.second"),
                capability_id("demo.third"),
            ])),
        );

        let outcome = filter
            .invoke_capability_batch(CapabilityBatchInvocation {
                invocations: vec![
                    invocation("demo.first", "input:first"),
                    invocation("demo.denied", "input:denied"),
                    invocation("demo.second", "input:second"),
                    invocation("demo.third", "input:third"),
                ],
                stop_on_first_suspension: true,
            })
            .await
            .expect("batch outcome");

        assert_eq!(outcome.outcomes.len(), 3);
        assert_eq!(
            denied_reason(&outcome.outcomes[1]),
            Some("surface_profile_denied")
        );
        assert!(outcome.stopped_on_suspension);
    }

    #[tokio::test]
    async fn stopped_inner_batch_truncates_denials_after_last_allowed_outcome() {
        let inner = Arc::new(SpyPort::default());
        *inner.batch_outcome.lock().expect("batch outcome lock") = Some(CapabilityBatchOutcome {
            outcomes: vec![approval_required("gate:first")],
            stopped_on_suspension: true,
        });
        let filter = CapabilitySurfaceProfileFilter::new(
            inner.clone(),
            Arc::new(CapabilityAllowSet::allowlist([capability_id("demo.first")])),
        );

        let outcome = filter
            .invoke_capability_batch(CapabilityBatchInvocation {
                invocations: vec![
                    invocation("demo.first", "input:first"),
                    invocation("demo.denied", "input:denied"),
                ],
                stop_on_first_suspension: true,
            })
            .await
            .expect("batch outcome");

        assert!(outcome.stopped_on_suspension);
        assert_eq!(outcome.outcomes.len(), 1);
        assert!(matches!(
            outcome.outcomes.as_slice(),
            [CapabilityOutcome::ApprovalRequired { .. }]
        ));
        let batches = inner.batches.lock().expect("batch lock");
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].invocations.len(), 1);
        assert!(batches[0].stop_on_first_suspension);
    }

    #[tokio::test]
    async fn surface_version_preserved() {
        let inner = Arc::new(SpyPort::default());
        *inner.surface.lock().expect("surface lock") = Some(VisibleCapabilitySurface {
            version: surface_version(),
            descriptors: vec![descriptor("demo.allowed"), descriptor("demo.denied")],
        });
        let filter = CapabilitySurfaceProfileFilter::new(
            inner,
            Arc::new(CapabilityAllowSet::allowlist([capability_id(
                "demo.allowed",
            )])),
        );

        let surface = filter
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("surface");

        assert_eq!(surface.version.as_str(), "surface-v1");
    }
}
