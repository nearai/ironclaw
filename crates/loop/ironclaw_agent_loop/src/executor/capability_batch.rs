use futures::{StreamExt, stream::FuturesUnordered};
use ironclaw_host_api::{
    resolution::{Resolution, ResolutionBatch, ToolVerdict},
    result_meta::FailureKind,
};
use ironclaw_loop_contracts::{BatchPolicyKind, LoopRequest, LoopRequestBatch};

use super::capabilities::CapabilityStage;
use super::capability_failure::recoverable_port_error_resolution;
use super::mapping::capability_port_error_is_terminal;
use super::pipeline::StageContext;

const MAX_PARALLEL_CAPABILITY_INVOCATIONS: usize = 4;

pub(super) enum InvokedCapabilityOutcome {
    Resolution(Resolution),
    TerminalError(ironclaw_loop_contracts::AgentLoopHostError),
}

pub(super) struct InvokedCapabilityBatch {
    pub(super) outcomes: Vec<InvokedCapabilityOutcome>,
    /// The bounded scheduler stopped admitting calls after a gate or typed
    /// cancellation. This is broader than the host's suspension-only flag.
    pub(super) truncated_launch_window: bool,
}

pub(super) struct InvokedCapabilityBatchError {
    pub(super) error: Box<ironclaw_loop_contracts::AgentLoopHostError>,
    pub(super) launched_count: usize,
}

impl InvokedCapabilityBatch {
    fn from_resolution_batch(batch: ResolutionBatch) -> Self {
        Self {
            outcomes: batch
                .resolutions
                .into_iter()
                .map(InvokedCapabilityOutcome::Resolution)
                .collect(),
            truncated_launch_window: batch.stopped_on_suspension,
        }
    }
}

fn resolution_stops_parallel_launch(resolution: &Resolution) -> bool {
    resolution.parks()
        || matches!(
            resolution,
            Resolution::Done(outcome)
                if matches!(
                    &outcome.verdict,
                    ToolVerdict::RecoverableFailure { error_kind, .. }
                        if *error_kind == FailureKind::Cancelled
                )
        )
}

impl CapabilityStage {
    pub(super) async fn invoke_batch(
        &self,
        ctx: StageContext<'_>,
        policy: BatchPolicyKind,
        invocations: Vec<LoopRequest>,
    ) -> Result<InvokedCapabilityBatch, InvokedCapabilityBatchError> {
        let ordered = invocations.len() >= 2
            && policy == BatchPolicyKind::Parallel
            && ctx.host.requires_ordered_batch_invocation(&invocations);
        if invocations.len() < 2 || policy != BatchPolicyKind::Parallel || ordered {
            return ctx
                .host
                .invoke_capability_batch(LoopRequestBatch {
                    invocations,
                    stop_on_first_suspension: matches!(policy, BatchPolicyKind::Sequential)
                        || ordered,
                })
                .await
                .map(InvokedCapabilityBatch::from_resolution_batch)
                .map_err(|error| InvokedCapabilityBatchError {
                    error: Box::new(error),
                    launched_count: 0,
                });
        }

        let invocation_count = invocations.len();
        let mut indexed_invocations = invocations.into_iter().enumerate();
        let invoke = |(index, invocation)| async move {
            (index, ctx.host.invoke_capability(invocation).await)
        };
        let mut pending = FuturesUnordered::new();
        let mut launched = 0_usize;
        for _ in 0..MAX_PARALLEL_CAPABILITY_INVOCATIONS {
            let Some(indexed_invocation) = indexed_invocations.next() else {
                break;
            };
            pending.push(invoke(indexed_invocation));
            launched += 1;
        }

        let mut outcomes = (0..invocation_count)
            .map(|_| None)
            .collect::<Vec<
                Option<Result<Resolution, ironclaw_loop_contracts::AgentLoopHostError>>,
            >>();
        let mut stop_launching = false;
        let mut terminal_error_seen = false;
        while let Some((index, result)) = pending.next().await {
            match &result {
                Ok(resolution) => stop_launching |= resolution_stops_parallel_launch(resolution),
                Err(error) => {
                    terminal_error_seen |= capability_port_error_is_terminal(error.kind);
                }
            }
            outcomes[index] = Some(result);

            if !stop_launching
                && !terminal_error_seen
                && let Some(indexed_invocation) = indexed_invocations.next()
            {
                pending.push(invoke(indexed_invocation));
                launched += 1;
            }
        }

        let mut normalized = Vec::with_capacity(launched);
        for outcome in outcomes.into_iter().take(launched) {
            let outcome = match outcome {
                Some(Ok(resolution)) => InvokedCapabilityOutcome::Resolution(resolution),
                Some(Err(error)) if capability_port_error_is_terminal(error.kind) => {
                    InvokedCapabilityOutcome::TerminalError(error)
                }
                Some(Err(error)) => {
                    InvokedCapabilityOutcome::Resolution(recoverable_port_error_resolution(error))
                }
                None => {
                    return Err(InvokedCapabilityBatchError {
                        error: Box::new(ironclaw_loop_contracts::AgentLoopHostError::new(
                            ironclaw_loop_contracts::AgentLoopHostErrorKind::Internal,
                            "parallel capability invocation completed without an indexed outcome",
                        )),
                        launched_count: launched,
                    });
                }
            };
            normalized.push(outcome);
        }

        Ok(InvokedCapabilityBatch {
            outcomes: normalized,
            truncated_launch_window: (stop_launching || terminal_error_seen)
                && launched < invocation_count,
        })
    }
}
