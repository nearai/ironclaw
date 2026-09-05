use ironclaw_host_api::process::{RuntimeProcessError, SandboxLoopWorkerSession};
use ironclaw_loop_contracts::*;
use ironclaw_turns::LoopMessageRef;
use std::collections::HashMap;

use crate::remote_host::protocol::*;
#[cfg(not(test))]
const WORKER_CANCELLATION_GRACE: std::time::Duration = std::time::Duration::from_secs(5);
#[cfg(test)]
const WORKER_CANCELLATION_GRACE: std::time::Duration = std::time::Duration::from_millis(25);

async fn cancellation_grace_elapsed(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

fn cancellation_grace_error() -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::Cancelled,
        "sandbox loop worker did not exit within the cancellation grace period",
    )
}

fn process_error(error: RuntimeProcessError) -> AgentLoopHostError {
    let error_kind = match error {
        RuntimeProcessError::Timeout(_) => "timeout",
        RuntimeProcessError::ExecutionFailed(_) => "execution_failed",
    };
    tracing::debug!(error_kind, "sandbox loop worker transport failed");
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::Unavailable,
        "sandbox loop worker transport failed",
    )
}
pub(super) fn wire_error_to_host_error(error: WireError) -> AgentLoopHostError {
    match error {
        WireError::Host(error) => error,
        WireError::Compaction(error) => {
            AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, error.to_string())
        }
        WireError::Protocol(detail) => {
            AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, detail)
        }
    }
}

pub(super) struct HostRpcState {
    max_model_calls: u32,
    max_capability_invocations: u32,
    model_calls: u32,
    prompt_builds: u32,
    capability_invocations: u32,
    resolve_messages: u32,
    last_model_iteration: Option<u32>,
    model_calls_this_iteration: u32,
    max_model_iteration: Option<u32>,
    max_model_calls_per_iteration: Option<u32>,
    model_usage: Option<LoopModelUsage>,
    /// Every `LoopModelMessage` (ref + role) the host has ever handed this
    /// run's worker, including the role it was issued with. `ResolveMessages`
    /// may only resolve pairs present here, so a worker cannot widen its
    /// visibility by guessing refs or re-labeling roles.
    issued_messages: HashMap<IssuedMessageKey, ()>,
}

/// Upper bound on tracked issued refs. A legal run issues far fewer; the cap
/// only stops a worker from spinning issue-producing calls to grow the ledger
/// without bound.
const MAX_TRACKED_ISSUED_REFS: usize = 4_096;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct IssuedMessageKey {
    content_ref: String,
    role: String,
}

impl IssuedMessageKey {
    fn of(message: &LoopModelMessage) -> Self {
        Self {
            content_ref: message.content_ref.as_str().to_string(),
            role: message.role.clone(),
        }
    }
}

impl HostRpcState {
    fn new(invocation: &LoopWorkerInvocation, settings: LoopWorkerSettings) -> Self {
        let profile = match invocation {
            LoopWorkerInvocation::Run(request) => &request.resolved_run_profile,
            LoopWorkerInvocation::Resume(request) => &request.resolved_run_profile,
        };
        Self {
            max_model_calls: profile.resource_budget_policy.max_model_calls,
            max_capability_invocations: profile.resource_budget_policy.max_capability_invocations,
            model_calls: 0,
            prompt_builds: 0,
            capability_invocations: 0,
            resolve_messages: 0,
            last_model_iteration: None,
            model_calls_this_iteration: 0,
            max_model_iteration: settings.default_iteration_limit,
            max_model_calls_per_iteration: settings
                .model_availability_attempts
                .map(|attempts| attempts.saturating_add(2)),
            model_usage: None,
            issued_messages: HashMap::new(),
        }
    }

    fn admit(&mut self, call: &HostCall) -> Result<(), WireError> {
        match call {
            HostCall::BuildPrompt(_) => {
                self.prompt_builds = checked_budget_increment(
                    self.prompt_builds,
                    1,
                    self.max_model_calls,
                    "prompt-build",
                )?;
            }
            HostCall::StreamModel(request) => {
                if self
                    .max_model_iteration
                    .is_some_and(|limit| request.iteration > limit)
                {
                    return Err(budget_error("model iteration"));
                }
                if self
                    .last_model_iteration
                    .is_some_and(|iteration| request.iteration < iteration)
                {
                    return Err(WireError::Host(AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "loop worker model iteration moved backwards",
                    )));
                }
                if self.last_model_iteration == Some(request.iteration) {
                    self.model_calls_this_iteration =
                        self.model_calls_this_iteration.saturating_add(1);
                } else {
                    self.last_model_iteration = Some(request.iteration);
                    self.model_calls_this_iteration = 1;
                }
                if self
                    .max_model_calls_per_iteration
                    .is_some_and(|limit| self.model_calls_this_iteration > limit)
                {
                    return Err(budget_error("model retry"));
                }
                self.model_calls = checked_budget_increment(
                    self.model_calls,
                    1,
                    self.max_model_calls,
                    "model-call",
                )?;
            }
            HostCall::InvokeCapability(_) => {
                self.capability_invocations = checked_budget_increment(
                    self.capability_invocations,
                    1,
                    self.max_capability_invocations,
                    "capability",
                )?;
            }
            HostCall::InvokeCapabilityBatch(request) => {
                let count = u32::try_from(request.invocations.len()).unwrap_or(u32::MAX);
                self.capability_invocations = checked_budget_increment(
                    self.capability_invocations,
                    count,
                    self.max_capability_invocations,
                    "capability",
                )?;
            }
            HostCall::ResolveMessages(_) => {
                self.resolve_messages = checked_budget_increment(
                    self.resolve_messages,
                    1,
                    self.resolve_budget(),
                    "resolve-messages",
                )?;
            }
            _ => {}
        }
        Ok(())
    }

    fn normalize_exit(&self, mut exit: LoopExit) -> LoopExit {
        match &mut exit {
            LoopExit::Completed(completed) => completed.model_usage = self.model_usage,
            LoopExit::Failed(failed) => {
                failed.model_usage = self.model_usage;
                failed.safe_summary = None;
            }
            LoopExit::Blocked(_) | LoopExit::Cancelled(_) => {}
        }
        exit
    }
    /// Bound on `ResolveMessages` admissions: one initial resolution plus one
    /// per completed capability call. A legal multi-tool batch resolves once
    /// per tool result, so the cap derives from the model and capability
    /// budgets rather than the model budget alone.
    fn resolve_budget(&self) -> u32 {
        self.max_model_calls
            .saturating_add(self.max_capability_invocations)
    }

    /// Record a message (ref + role) the host has issued to this run's worker.
    fn issue_message(&mut self, message: &LoopModelMessage) {
        if self.issued_messages.len() >= MAX_TRACKED_ISSUED_REFS {
            return;
        }
        self.issued_messages
            .insert(IssuedMessageKey::of(message), ());
    }

    /// Record every message in a response payload the worker just received.
    fn issue_messages<'a>(&mut self, messages: impl IntoIterator<Item = &'a LoopModelMessage>) {
        for message in messages {
            self.issue_message(message);
        }
    }

    /// Record resolvable refs carried by a `LoadContext` bundle response.
    fn issue_context_bundle(&mut self, bundle: &LoopContextBundle) {
        for message in bundle
            .identity_messages
            .iter()
            .chain(bundle.messages.iter())
        {
            let Some(content_ref) = message.message_ref.as_ref() else {
                continue;
            };
            self.issue_message(&LoopModelMessage {
                role: message.role.clone(),
                content_ref: content_ref.clone(),
            });
        }
        for snippet in bundle
            .instruction_snippets
            .iter()
            .chain(bundle.memory_snippets.iter())
        {
            let Ok(content_ref) = LoopMessageRef::new(snippet.snippet_ref.clone()) else {
                continue;
            };
            self.issue_message(&LoopModelMessage {
                role: "system".to_string(),
                content_ref,
            });
        }
    }

    /// Record message refs carried by a polled input batch.
    fn issue_input_batch(&mut self, batch: &LoopInputBatch) {
        for input in &batch.inputs {
            let message_ref = match input {
                LoopInput::UserMessage { message_ref }
                | LoopInput::FollowUp { message_ref }
                | LoopInput::Steering { message_ref }
                | LoopInput::SubagentSettled { message_ref, .. } => message_ref,
                LoopInput::Interrupt { .. }
                | LoopInput::Cancel { .. }
                | LoopInput::GateResolved { .. }
                | LoopInput::CapabilitySurfaceChanged { .. } => continue,
            };
            self.issue_message(&LoopModelMessage {
                role: "user".to_string(),
                content_ref: message_ref.clone(),
            });
        }
    }

    /// Check a `ResolveMessages` request against the issued ledger. Empty
    /// requests are rejected: the contract scopes resolution to refs the host
    /// already issued, and an empty request would otherwise resolve the run's
    /// whole context window instead.
    fn authorize_resolve_messages(
        &self,
        request: &ResolveMessagesRequest,
    ) -> Result<(), WireError> {
        if request.messages.is_empty() {
            return Err(WireError::Host(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "sandbox loop worker must name the message refs to resolve",
            )));
        }
        for message in &request.messages {
            if !self
                .issued_messages
                .contains_key(&IssuedMessageKey::of(message))
            {
                return Err(WireError::Host(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::PolicyDenied,
                    "sandbox loop worker may only resolve message refs issued to this run",
                )));
            }
        }
        Ok(())
    }
}

fn checked_budget_increment(
    current: u32,
    increment: u32,
    maximum: u32,
    operation: &'static str,
) -> Result<u32, WireError> {
    let next = current.saturating_add(increment);
    if next > maximum {
        return Err(budget_error(operation));
    }
    Ok(next)
}

fn budget_error(operation: &'static str) -> WireError {
    WireError::Host(AgentLoopHostError::new(
        AgentLoopHostErrorKind::BudgetExceeded,
        format!("sandbox loop worker exceeded the host-owned {operation} budget"),
    ))
}
#[allow(unreachable_pub)]
pub(super) async fn dispatch_host_call(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    content: Option<&dyn LoopMessageContentPort>,
    visibility: WorkerContentVisibility,
    call: HostCall,
    state: &mut HostRpcState,
) -> Result<serde_json::Value, WireError> {
    state.admit(&call)?;
    macro_rules! host_call {
        ($future:expr) => {{
            let value = $future.await.map_err(WireError::Host)?;
            serde_json::to_value(value).map_err(|error| {
                WireError::Protocol(format!("host response serialization failed: {error}"))
            })
        }};
    }

    match call {
        HostCall::ResolveMessages(request) => {
            let content = match (visibility, content) {
                (WorkerContentVisibility::Resolved, Some(content)) => content,
                (WorkerContentVisibility::Resolved, None) => {
                    return Err(WireError::Host(AgentLoopHostError::new(
                        AgentLoopHostErrorKind::PolicyDenied,
                        "sandbox loop worker content resolution is unavailable on this host",
                    )));
                }
                (WorkerContentVisibility::Blind, _) => {
                    return Err(WireError::Host(AgentLoopHostError::new(
                        AgentLoopHostErrorKind::PolicyDenied,
                        "sandbox loop worker is not permitted to resolve message content",
                    )));
                }
            };
            state.authorize_resolve_messages(&request)?;
            let resolved = content
                .resolve_message_content(request.messages)
                .await
                .map_err(WireError::Host)?;
            let wire: Vec<WireResolvedModelMessage> = resolved
                .into_iter()
                .map(|message| WireResolvedModelMessage {
                    role: message.role,
                    content_ref: message.content_ref,
                    content: message.content,
                    tool_result: message.tool_result.map(|result| WireResolvedToolResult {
                        provider_call_id: result.provider_call_id,
                        content: result.content,
                    }),
                })
                .collect();
            serde_json::to_value(wire).map_err(|error| {
                WireError::Protocol(format!("resolved message serialization failed: {error}"))
            })
        }
        HostCall::LoadContext(request) => {
            let bundle = host
                .load_loop_context(request)
                .await
                .map_err(WireError::Host)?;
            state.issue_context_bundle(&bundle);
            serde_json::to_value(WireLoopContextBundle::from(bundle)).map_err(|error| {
                WireError::Protocol(format!(
                    "loop context response serialization failed: {error}"
                ))
            })
        }
        HostCall::BuildPrompt(request) => {
            let bundle = host
                .build_prompt_bundle(request)
                .await
                .map_err(WireError::Host)?;
            state.issue_messages(&bundle.messages);
            serde_json::to_value(bundle).map_err(|error| {
                WireError::Protocol(format!("host response serialization failed: {error}"))
            })
        }
        HostCall::PollInputs { after, limit } => {
            let batch = host
                .poll_inputs(after, limit)
                .await
                .map_err(WireError::Host)?;
            state.issue_input_batch(&batch);
            serde_json::to_value(batch).map_err(|error| {
                WireError::Protocol(format!("host response serialization failed: {error}"))
            })
        }
        HostCall::AckInputs(tokens) => host_call!(host.ack_inputs(tokens)),
        HostCall::StreamModel(request) => {
            let response = host.stream_model(request).await.map_err(WireError::Host)?;
            state.model_usage = LoopModelUsage::merge_optional(state.model_usage, response.usage);
            serde_json::to_value(response).map_err(|error| {
                WireError::Protocol(format!("host response serialization failed: {error}"))
            })
        }
        HostCall::RegisterProviderToolCall(request) => {
            host_call!(host.register_provider_tool_call(request))
        }
        HostCall::VisibleCapabilities(request) => {
            let surface = host
                .visible_capabilities(request)
                .await
                .map_err(WireError::Host)?;
            serde_json::to_value(WireVisibleCapabilitySurface::from(surface)).map_err(|error| {
                WireError::Protocol(format!(
                    "visible capability surface serialization failed: {error}"
                ))
            })
        }
        HostCall::InvokeCapability(request) => host_call!(host.invoke_capability(request)),
        HostCall::InvokeCapabilityBatch(request) => {
            host_call!(host.invoke_capability_batch(request))
        }
        HostCall::BeginAssistantDraft(request) => host_call!(host.begin_assistant_draft(request)),
        HostCall::UpdateAssistantDraft(request) => host_call!(host.update_assistant_draft(request)),
        HostCall::FinalizeAssistantMessage(request) => {
            let message_ref = host
                .finalize_assistant_message(request)
                .await
                .map_err(WireError::Host)?;
            state.issue_message(&LoopModelMessage {
                role: "assistant".to_string(),
                content_ref: message_ref.clone(),
            });
            serde_json::to_value(message_ref).map_err(|error| {
                WireError::Protocol(format!("host response serialization failed: {error}"))
            })
        }
        HostCall::AppendCapabilityResultRef(request) => {
            let message_ref = host
                .append_capability_result_ref(*request)
                .await
                .map_err(WireError::Host)?;
            state.issue_message(&LoopModelMessage {
                role: "tool_result_reference".to_string(),
                content_ref: message_ref.clone(),
            });
            serde_json::to_value(message_ref).map_err(|error| {
                WireError::Protocol(format!("host response serialization failed: {error}"))
            })
        }
        HostCall::Checkpoint(request) => host_call!(host.checkpoint(request)),
        HostCall::StageCheckpointPayload(request) => {
            host_call!(host.stage_checkpoint_payload(request))
        }
        HostCall::LoadCheckpointPayload(request) => {
            let payload = host
                .load_checkpoint_payload(request)
                .await
                .map_err(WireError::Host)?;
            serde_json::to_value(WireLoadedCheckpointPayload::from(payload)).map_err(|error| {
                WireError::Protocol(format!("checkpoint payload serialization failed: {error}"))
            })
        }
        HostCall::EmitProgress(event) => host_call!(host.emit_loop_progress(event)),
        HostCall::Compact(request) => {
            let value = host
                .compact_loop_context(request)
                .await
                .map_err(WireError::Compaction)?;
            if let LoopCompactionOutcome::Compacted(response) = &value {
                let Ok(content_ref) =
                    LoopMessageRef::new(format!("msg:summary-{}", response.summary_artifact_id))
                else {
                    return Err(WireError::Protocol(
                        "compaction summary ref could not be represented".to_string(),
                    ));
                };
                state.issue_message(&LoopModelMessage {
                    role: "assistant".to_string(),
                    content_ref,
                });
            }
            serde_json::to_value(value).map_err(|error| {
                WireError::Protocol(format!("compaction response serialization failed: {error}"))
            })
        }
    }
}

pub async fn serve_loop_worker(
    session: &mut dyn SandboxLoopWorkerSession,
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    invocation: LoopWorkerInvocation,
    settings: LoopWorkerSettings,
    content: Option<&dyn LoopMessageContentPort>,
    visibility: WorkerContentVisibility,
) -> Result<LoopWorkerOutcome, AgentLoopHostError> {
    let mut rpc_state = HostRpcState::new(&invocation, settings);
    let bootstrap = LoopWorkerBootstrap {
        wire_version: LOOP_WORKER_WIRE_VERSION,
        run_context: host.run_context().clone(),
        settings,
        invocation,
        tool_definitions: host.tool_definitions()?,
        current_visible_capabilities: host
            .current_visible_capabilities()?
            .map(WireVisibleCapabilitySurface::from)
            .map(serde_json::to_value)
            .transpose()
            .map_err(|error| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    format!("visible capability bootstrap serialization failed: {error}"),
                )
            })?,
        content_visibility: visibility,
    };
    session
        .send(encode(&HostFrame::Bootstrap(Box::new(bootstrap)))?)
        .await
        .map_err(process_error)?;

    let mut cancellation_sent = false;
    let mut cancellation_deadline = None;
    loop {
        let bytes = tokio::select! {
            frame = session.receive() => frame
                .map_err(process_error)?
                .ok_or_else(|| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Unavailable,
                        "sandbox loop worker exited before returning a loop outcome",
                    )
                })?,
            signal = host.cancellation_requested(), if !cancellation_sent => {
                session
                    .send(encode(&HostFrame::Cancel(signal))?)
                    .await
                    .map_err(process_error)?;
                cancellation_sent = true;
                cancellation_deadline =
                    Some(tokio::time::Instant::now() + WORKER_CANCELLATION_GRACE);
                continue;
            }
            _ = cancellation_grace_elapsed(cancellation_deadline) => {
                return Err(cancellation_grace_error());
            }
        };
        match decode::<WorkerFrame>(&bytes)? {
            WorkerFrame::Outcome(outcome) => {
                session
                    .send(encode(&HostFrame::OutcomeAck)?)
                    .await
                    .map_err(process_error)?;
                return Ok(match outcome {
                    LoopWorkerOutcome::Exit(exit) => {
                        LoopWorkerOutcome::Exit(rpc_state.normalize_exit(exit))
                    }
                    LoopWorkerOutcome::Failed(failure) => LoopWorkerOutcome::Failed(failure),
                });
            }
            WorkerFrame::HostRequest(request) => {
                let mut dispatch = Box::pin(dispatch_host_call(
                    host,
                    content,
                    visibility,
                    request.call,
                    &mut rpc_state,
                ));
                let result = loop {
                    tokio::select! {
                        result = &mut dispatch => break result,
                        signal = host.cancellation_requested(), if !cancellation_sent => {
                            session
                                .send(encode(&HostFrame::Cancel(signal))?)
                                .await
                                .map_err(process_error)?;
                            cancellation_sent = true;
                            cancellation_deadline =
                                Some(tokio::time::Instant::now() + WORKER_CANCELLATION_GRACE);
                        }
                        _ = cancellation_grace_elapsed(cancellation_deadline) => {
                            return Err(cancellation_grace_error());
                        }
                    }
                };
                session
                    .send(encode(&HostFrame::HostResponse(HostResponseFrame {
                        id: request.id,
                        result,
                    }))?)
                    .await
                    .map_err(process_error)?;
            }
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub(in crate::remote_host) fn rpc_state(
        max_model_calls: u32,
        max_capability_invocations: u32,
    ) -> HostRpcState {
        HostRpcState {
            max_model_calls,
            max_capability_invocations,
            model_calls: 0,
            prompt_builds: 0,
            capability_invocations: 0,
            resolve_messages: 0,
            last_model_iteration: None,
            model_calls_this_iteration: 0,
            max_model_iteration: Some(1),
            max_model_calls_per_iteration: Some(2),
            model_usage: None,
            issued_messages: HashMap::new(),
        }
    }

    fn model_call(iteration: u32) -> HostCall {
        HostCall::StreamModel(LoopModelRequest {
            messages: Vec::new(),
            inline_messages: Vec::new(),
            surface_version: None,
            model_preference: None,
            fallback_index: 0,
            iteration,
            capability_view: None,
            tool_choice: None,
        })
    }

    #[test]
    fn host_rpc_state_enforces_model_call_iteration_and_retry_limits() {
        let mut state = rpc_state(2, 4);
        state.admit(&model_call(0)).expect("first call");
        state.admit(&model_call(0)).expect("one retry");

        let retry = state.admit(&model_call(0)).expect_err("retry limit");
        assert!(matches!(
            retry,
            WireError::Host(AgentLoopHostError {
                kind: AgentLoopHostErrorKind::BudgetExceeded,
                ..
            })
        ));

        let mut state = rpc_state(4, 4);
        let iteration = state.admit(&model_call(2)).expect_err("iteration limit");
        assert!(matches!(
            iteration,
            WireError::Host(AgentLoopHostError {
                kind: AgentLoopHostErrorKind::BudgetExceeded,
                ..
            })
        ));
    }

    #[test]
    fn host_rpc_state_replaces_worker_reported_model_usage() {
        let trusted = LoopModelUsage {
            input_tokens: 10,
            output_tokens: 3,
            cache_read_input_tokens: 2,
            cache_creation_input_tokens: 1,
        };
        let mut state = rpc_state(4, 4);
        state.model_usage = Some(trusted);
        let forged = LoopExit::Completed(LoopCompleted {
            completion_kind: LoopCompletionKind::NoReply,
            reply_message_refs: Vec::new(),
            result_refs: Vec::new(),
            final_checkpoint_id: None,
            model_usage: Some(LoopModelUsage {
                input_tokens: u32::MAX,
                output_tokens: u32::MAX,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            }),
            exit_id: ironclaw_host_api::turn::LoopExitId::new("exit:worker-forged-usage")
                .expect("exit id"),
        });

        let LoopExit::Completed(normalized) = state.normalize_exit(forged) else {
            panic!("expected completed exit");
        };
        assert_eq!(normalized.model_usage, Some(trusted));
    }

    #[test]
    fn host_rpc_state_discards_worker_reported_failure_summary() {
        let forged = LoopExit::Failed(LoopFailed {
            reason_kind: LoopFailureKind::ModelError,
            checkpoint_id: None,
            model_usage: None,
            exit_id: ironclaw_host_api::turn::LoopExitId::new("exit:worker-forged-summary")
                .expect("exit id"),
            explanation_message_refs: Vec::new(),
            safe_summary: Some(
                ironclaw_host_api::turn::SanitizedFailure::new("model_error")
                    .expect("failure")
                    .with_detail("worker-controlled diagnostic"),
            ),
        });

        let LoopExit::Failed(normalized) = rpc_state(4, 4).normalize_exit(forged) else {
            panic!("expected failed exit");
        };
        assert_eq!(normalized.safe_summary, None);
    }
}
