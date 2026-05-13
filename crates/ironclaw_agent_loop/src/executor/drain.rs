//! Input-port draining helpers: cancellation observation, steering
//! drain, follow-up drain, and control-input side-effect application.

use ironclaw_turns::{
    LoopGateRef,
    run_profile::{AgentLoopDriverHost, LoopInput, ProcessHandleSummary},
};

use crate::{
    planner::AgentLoopPlanner,
    state::{CheckpointKind, LoopExecutionState},
};

use super::util::INPUT_POLL_LIMIT;
use super::{
    AgentLoopExecutorError, CancelledKind, CanonicalAgentLoopExecutor, HostStage, LoopExit,
};

/// Outcome of a follow-up drain poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FollowupDrainOutcome {
    /// A `FollowUp` was acked; the loop must continue (no `Final` checkpoint).
    /// Any GateResolved / CapabilitySurfaceChanged inputs in the same page were
    /// applied to state in-place as idempotent side effects.
    FollowUpConsumed,
    /// A `Cancel` or `Interrupt` was observed in the drain page. The page
    /// has NOT been acked — `drain_followup` carries the `next_cursor` back
    /// to the caller, which must take the `Final` checkpoint and only then
    /// ack the page. Sibling control side effects in the same page were
    /// applied in place. Acking before the checkpoint would leave the
    /// cancel consumed but the run un-persisted on a checkpoint failure.
    TerminalCancel {
        next_cursor: ironclaw_turns::run_profile::LoopInputCursor,
    },
    /// Drained `INPUT_POLL_LIMIT` consecutive control-only pages without
    /// reaching a definitive answer. All control side effects were applied
    /// and their pages were acked, but we cannot conclude the queue is
    /// empty — a genuine FollowUp may be sitting on a later page. The
    /// caller MUST NOT take the `Final` checkpoint and MUST NOT exit
    /// `Completed`; it should advance the iteration and let the next tick
    /// continue draining. Returning `Empty` here would strand a FollowUp
    /// sitting past page `INPUT_POLL_LIMIT`.
    ControlPending,
    /// Queue was empty (or contained only GateResolved / SurfaceChanged that
    /// were applied + acked); the loop completes naturally.
    Empty,
}

impl CanonicalAgentLoopExecutor {
    /// If the planner's drain strategy opts in, poll the input queue and
    /// return the followup-drain outcome. The caller decides whether to
    /// take the `Final` checkpoint (Empty) or continue the outer loop
    /// (FollowUpConsumed / ControlPending).
    pub(super) async fn drain_followup_if_planner_asks(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
    ) -> Result<(LoopExecutionState, FollowupDrainOutcome), AgentLoopExecutorError> {
        if planner.drain().drain_followup(&state).await {
            self.drain_followup(host, state).await
        } else {
            Ok((state, FollowupDrainOutcome::Empty))
        }
    }

    pub(super) async fn observe_cancellation(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
    ) -> Result<(LoopExecutionState, Option<LoopExit>), AgentLoopExecutorError> {
        // Page past control-only pages just like `drain_followup`. Polling
        // exactly once would let a queued `Cancel`/`Interrupt` on a later
        // page stay invisible until the loop produced one more reply or
        // ran extra tools.
        //
        // Loop up to `INPUT_POLL_LIMIT` rounds — same defense-in-depth
        // bound as `drain_followup`. The loop terminates on:
        //   - terminal input → checkpoint-then-ack-then-exit
        //   - empty page    → no cancel pending, return None
        //   - user-facing input (UserMessage / FollowUp / Steering) → leave
        //     un-acked for the dedicated drain handler, return None
        for _ in 0..INPUT_POLL_LIMIT {
            let batch = host
                .poll_inputs(state.input_cursor.clone(), INPUT_POLL_LIMIT)
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                    stage: HostStage::Input,
                })?;
            // Apply control side effects (GateResolved,
            // CapabilitySurfaceChanged) in-page as idempotent state
            // mutations. Pages are atomic — the cursor is page-granular
            // — so we can't partial-ack between a control event and a
            // user-facing event in the same page.
            apply_control_side_effects(&mut state, &batch.inputs);

            // Cancel / Interrupt are terminal: take `Final` first, then
            // ack the page only once the checkpoint is durable. Acking
            // before the checkpoint would consume the cancel without
            // persisting state, so a retried run would observe
            // `state.input_cursor` past the cancel and never re-poll
            // it.
            if batch.inputs.iter().any(|input| {
                matches!(
                    input,
                    LoopInput::Cancel { .. } | LoopInput::Interrupt { .. }
                )
            }) {
                // Advance the cursor on `state` BEFORE checkpointing so
                // the durable record names the next-unprocessed position.
                // Ack-then-checkpoint would let a checkpoint write
                // failure leave the host with the page dropped but the
                // only durable cursor still pointing at the cancel.
                // Checkpoint-with-advanced-cursor-then-ack means a
                // checkpoint failure bubbles up before the host drops
                // the page; ack failure after a successful checkpoint
                // is benign (next iteration's poll skips
                // already-processed positions).
                state.input_cursor = batch.next_cursor.clone();
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                host.ack_inputs(batch.next_cursor).await.map_err(|_| {
                    AgentLoopExecutorError::HostUnavailable {
                        stage: HostStage::Input,
                    }
                })?;
                let exit = LoopExit::Cancelled(CancelledKind {
                    interrupted_message_refs: checked.assistant_refs.clone(),
                });
                return Ok((checked, Some(exit)));
            }

            // Empty page → no cancel pending; return.
            if batch.inputs.is_empty() {
                return Ok((state, None));
            }

            // User-facing inputs (UserMessage / FollowUp / Steering)
            // belong to dedicated drain handlers. Leave the page un-acked
            // and return `None` so the iteration proceeds. Any control
            // side effects in the same page were already applied above
            // — those mutations are idempotent and survive the next
            // drain handler's re-poll of the same cursor.
            let has_user_facing = batch.inputs.iter().any(|input| {
                matches!(
                    input,
                    LoopInput::UserMessage { .. }
                        | LoopInput::FollowUp { .. }
                        | LoopInput::Steering { .. }
                )
            });
            if has_user_facing {
                return Ok((state, None));
            }

            // Control-only page: side effects were applied above.
            // Checkpoint with the advanced cursor and applied side
            // effects BEFORE the host ack — otherwise a crash between
            // ack and the next durable checkpoint would resume from an
            // older checkpoint pointing at the already-dropped page,
            // losing the GateResolved / CapabilitySurfaceChanged side
            // effects.
            state.input_cursor = batch.next_cursor.clone();
            state = self
                .checkpoint(host, state, CheckpointKind::BeforeModel)
                .await?;
            host.ack_inputs(batch.next_cursor).await.map_err(|_| {
                AgentLoopExecutorError::HostUnavailable {
                    stage: HostStage::Input,
                }
            })?;
        }
        // Defense-in-depth: `INPUT_POLL_LIMIT` consecutive control-only
        // pages without a terminal, empty, or user-facing page. Return
        // `None` so the outer loop makes progress — the next tick's
        // `observe_cancellation` will pick up where we left off.
        Ok((state, None))
    }

    pub(super) async fn drain_steering(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
    ) -> Result<LoopExecutionState, AgentLoopExecutorError> {
        let batch = host
            .poll_inputs(state.input_cursor.clone(), INPUT_POLL_LIMIT)
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                stage: HostStage::Input,
            })?;
        // Pages are atomic. Apply control side effects in-page
        // (gate-resolved → clear last_gate; surface-changed → drop cached
        // surface_version). If a user-facing steering message is present
        // in the same page, ack so the loop makes progress. If
        // Cancel/Interrupt is also present, don't ack — the next
        // iteration's `observe_cancellation` polls the same cursor and
        // exits Cancelled. The FollowUp case stays un-acked here so the
        // dedicated post-reply drain handler can consume it.
        apply_control_side_effects(&mut state, &batch.inputs);
        let has_terminal = batch.inputs.iter().any(|input| {
            matches!(
                input,
                LoopInput::Cancel { .. } | LoopInput::Interrupt { .. }
            )
        });
        let has_steering = batch.inputs.iter().any(|input| {
            matches!(
                input,
                LoopInput::UserMessage { .. } | LoopInput::Steering { .. }
            )
        });
        let has_followup = batch
            .inputs
            .iter()
            .any(|input| matches!(input, LoopInput::FollowUp { .. }));
        if has_terminal || has_followup {
            // Don't ack — leave the page to observe_cancellation /
            // drain_followup. Control side effects were already applied.
            return Ok(state);
        }
        if has_steering {
            // Checkpoint with the advanced cursor (and any applied
            // control side effects) BEFORE acking. Ack-then-checkpoint
            // would let a worker crash resume from an older checkpoint
            // re-polling a page the host has already discarded, losing
            // the steering message and any sibling control side
            // effects.
            state.input_cursor = batch.next_cursor.clone();
            state = self
                .checkpoint(host, state, CheckpointKind::BeforeModel)
                .await?;
            host.ack_inputs(batch.next_cursor).await.map_err(|_| {
                AgentLoopExecutorError::HostUnavailable {
                    stage: HostStage::Input,
                }
            })?;
        }
        Ok(state)
    }

    pub(super) async fn drain_followup(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
    ) -> Result<(LoopExecutionState, FollowupDrainOutcome), AgentLoopExecutorError> {
        // Keep draining follow-up pages until either a FollowUp /
        // terminal input is found, or the queue is genuinely empty.
        // Returning `Empty` after acking a control-only page would
        // silently drop a FollowUp on a *later* page. Pages stay atomic
        // (we still ack one at a time); we just keep polling.
        //
        // Defense-in-depth bound: at most `INPUT_POLL_LIMIT` poll rounds
        // per call so a misbehaving host that returns an infinite stream
        // of control-only pages can't spin forever inside one drain.
        for _ in 0..INPUT_POLL_LIMIT {
            let batch = host
                .poll_inputs(state.input_cursor.clone(), INPUT_POLL_LIMIT)
                .await
                .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                    stage: HostStage::Input,
                })?;
            // A fresh `UserMessage` or `Steering` arriving just as the
            // loop would otherwise complete must be treated as
            // follow-up-equivalent — it's user-facing input the next
            // iteration owes a reply to. Matching only `FollowUp` would
            // let a post-reply `UserMessage` fall through to the
            // control-only branch and get acked away.
            let has_followup = batch.inputs.iter().any(|input| {
                matches!(
                    input,
                    LoopInput::FollowUp { .. }
                        | LoopInput::UserMessage { .. }
                        | LoopInput::Steering { .. }
                )
            });
            let has_terminal = batch.inputs.iter().any(|input| {
                matches!(
                    input,
                    LoopInput::Cancel { .. } | LoopInput::Interrupt { .. }
                )
            });
            // Master spec §8 step 2: pages are atomic — the
            // `LoopInputPort` cursor is page-granular, so a mixed page
            // (FollowUp + GateResolved + SurfaceChanged) must be acked
            // as a whole. Apply control side effects in-page as
            // idempotent state mutations, then ack. A
            // refuse-to-ack-on-mixed-page approach would livelock on
            // any page where a control event sits with a user-facing
            // input.
            apply_control_side_effects(&mut state, &batch.inputs);
            if has_terminal {
                // Cancel/Interrupt is terminal. Do NOT ack here. Carry
                // the un-applied cursor back to the caller, which takes
                // `Final` and then acks. Ack-first would leave the
                // cancel consumed but the run state un-persisted on a
                // checkpoint failure. Sibling control side effects
                // were already applied via `apply_control_side_effects`.
                return Ok((
                    state,
                    FollowupDrainOutcome::TerminalCancel {
                        next_cursor: batch.next_cursor,
                    },
                ));
            }
            if has_followup {
                // Durably checkpoint with the advanced cursor BEFORE
                // the host ack. The caller's outer loop won't checkpoint
                // until after another observe_cancellation /
                // drain_steering cycle plus a model invocation, so
                // ack-then-crash would lose the GateResolved /
                // CapabilitySurfaceChanged side effects sharing the
                // mixed page.
                state.input_cursor = batch.next_cursor.clone();
                state = self
                    .checkpoint(host, state, CheckpointKind::BeforeModel)
                    .await?;
                host.ack_inputs(batch.next_cursor).await.map_err(|_| {
                    AgentLoopExecutorError::HostUnavailable {
                        stage: HostStage::Input,
                    }
                })?;
                return Ok((state, FollowupDrainOutcome::FollowUpConsumed));
            }
            // No user-facing or terminal inputs in this page.
            if batch.inputs.is_empty() {
                // Queue is genuinely drained. The caller's next step is
                // the `Final` checkpoint.
                return Ok((state, FollowupDrainOutcome::Empty));
            }
            // Control-only page: side effects were just applied.
            // Durably checkpoint with the advanced cursor and applied
            // side effects BEFORE the ack — otherwise ack-then-crash
            // would resume from an older checkpoint re-polling a page
            // the host has dropped, permanently losing the GateResolved
            // / CapabilitySurfaceChanged effects.
            state.input_cursor = batch.next_cursor.clone();
            state = self
                .checkpoint(host, state, CheckpointKind::BeforeModel)
                .await?;
            host.ack_inputs(batch.next_cursor).await.map_err(|_| {
                AgentLoopExecutorError::HostUnavailable {
                    stage: HostStage::Input,
                }
            })?;
        }
        // `INPUT_POLL_LIMIT` consecutive control-only pages were acked
        // but we never saw a definitive "empty" page or a user-facing
        // input. Collapsing this into `Empty` would let the caller
        // Final-checkpoint and exit `Completed` even with a real
        // FollowUp sitting on a later page. Return `ControlPending` so
        // the caller continues the loop.
        Ok((state, FollowupDrainOutcome::ControlPending))
    }
}

/// Apply idempotent control-input side effects to `state`. Cancel and
/// Interrupt are NOT handled here — the caller decides terminal exit.
pub(super) fn apply_control_side_effects(state: &mut LoopExecutionState, inputs: &[LoopInput]) {
    for input in inputs {
        match input {
            LoopInput::GateResolved { gate_ref } if state.last_gate.as_ref() == Some(gate_ref) => {
                state.last_gate = None;
            }
            LoopInput::CapabilitySurfaceChanged { .. } => {
                // Drop the cached surface_version so the next iteration's
                // `visible_capabilities` re-fetch picks up the new host
                // snapshot (master spec §10).
                state.surface_version = None;
            }
            _ => {}
        }
    }
}

/// Convert a `LoopProcessRef` (prefix `process:`) to a `LoopGateRef`
/// (prefix `gate:`) so a `SpawnedProcess` outcome can flow through the
/// existing gate-handling path.
///
/// The skeleton has no `LoopBlockedKind::WaitingForProcess` variant yet;
/// this synthesizes a `gate:proc-<token>` ref so the executor can take
/// `BeforeBlock` and surface a `Blocked { gate_ref }` exit. The runner
/// resumes when the process emits its completion event via
/// `LoopInputPort` (`GateResolved` with the same gate ref, or
/// `CapabilitySurfaceChanged`).
pub(super) fn process_ref_to_gate_ref(
    handle: &ProcessHandleSummary,
) -> Result<LoopGateRef, AgentLoopExecutorError> {
    let token = handle
        .process_ref
        .as_str()
        .strip_prefix("process:")
        .unwrap_or(handle.process_ref.as_str());
    LoopGateRef::new(format!("gate:proc-{token}")).map_err(|_| {
        AgentLoopExecutorError::PlannerContract {
            detail: "spawned-process handle could not be projected to a gate ref",
        }
    })
}
