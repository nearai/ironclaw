//! Host-owned structured-output finalization and durable evidence.
//!
//! This coordinator is intentionally outside `ironclaw_agent_loop`.  The loop
//! supplies its ordinary terminal candidate; this layer owns the one guarded
//! finalizer call, lease fence, immutable thread record, and supplemental usage
//! snapshot that the runner merges into the exit.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{
    output::OutputContract,
    turn::{LoopMessageRef, TurnLeaseToken},
};
use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, AssistantReply, LoopModelUsage, LoopRunContext,
    PromptContextTokenBudget, SystemInferenceIdentity, SystemInferencePort, SystemInferenceRequest,
    SystemInferenceTaskId, SystemPromptSource, SystemTaskKind,
};
use ironclaw_threads::{
    MessageKind, MessageStatus, PublishStructuredFinalizationMessageRequest,
    PutStructuredFinalizationRequest, ReadStructuredFinalizationRequest, SessionThreadService,
    StructuredFinalizationAccounting, StructuredFinalizationRecord, StructuredFinalizationUsage,
    ThreadMessageId, ThreadScope,
};
use ironclaw_turns::AgentTurnSpawnTreeRuntimePort;

// Bounds the complete structured-finalization operation at 500 seconds: enough
// for four default attempts at 60 seconds to first semantic progress plus a
// 60-second semantic-idle window each, with 8.75 seconds of maximum jittered
// backoff. Active semantic progress has no per-attempt total cap; this outer
// deadline remains authoritative for a finite headless run. The process
// supervisor heartbeat and durable lease fences still apply: a stale executor
// is rejected before durable publication.
const FINALIZATION_DEADLINE_MS: u64 = 500_000;

#[async_trait]
pub(crate) trait StructuredFinalizationPort: Send + Sync {
    async fn finalize_terminal_reply(
        &self,
        message_ref: &LoopMessageRef,
    ) -> Result<(), AgentLoopHostError>;
    fn supplemental_model_usage(&self) -> Option<LoopModelUsage>;
}

pub(crate) struct StructuredFinalizationContextLimits {
    pub(crate) max_messages: usize,
    pub(crate) token_budget: PromptContextTokenBudget,
}

pub(crate) struct StructuredFinalizationCoordinator<S>
where
    S: SessionThreadService + ?Sized,
{
    thread_service: Arc<S>,
    thread_scope: ThreadScope,
    run_context: LoopRunContext,
    inference: Arc<dyn SystemInferencePort>,
    runtime: Arc<dyn AgentTurnSpawnTreeRuntimePort>,
    lease_token: TurnLeaseToken,
    max_context_messages: usize,
    prompt_context_budget: PromptContextTokenBudget,
    supplemental_usage: Mutex<Option<LoopModelUsage>>,
}

impl<S> StructuredFinalizationCoordinator<S>
where
    S: SessionThreadService + ?Sized,
{
    pub(crate) fn new(
        thread_service: Arc<S>,
        thread_scope: ThreadScope,
        run_context: LoopRunContext,
        inference: Arc<dyn SystemInferencePort>,
        runtime: Arc<dyn AgentTurnSpawnTreeRuntimePort>,
        lease_token: TurnLeaseToken,
        context_limits: StructuredFinalizationContextLimits,
    ) -> Self {
        Self {
            thread_service,
            thread_scope,
            run_context,
            inference,
            runtime,
            lease_token,
            max_context_messages: context_limits.max_messages.max(1),
            prompt_context_budget: context_limits.token_budget,
            supplemental_usage: Mutex::new(None),
        }
    }

    async fn finalize_candidate(
        &self,
        candidate: &AssistantReply,
    ) -> Result<String, AgentLoopHostError> {
        ironclaw_loop_host::ensure_run_lease_is_current(
            self.runtime.as_ref(),
            &self.run_context,
            self.lease_token,
        )
        .await?;
        let contract = match &self.run_context.output_contract {
            OutputContract::JsonSchema { .. } | OutputContract::JsonObject => {
                self.run_context.output_contract.clone()
            }
            OutputContract::AssistantMessage => {
                return Err(host_error(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "structured finalization requires a structured output contract",
                ));
            }
        };
        let (contract_name, schema_digest) = contract_identity(&contract)?;
        let read_request = ReadStructuredFinalizationRequest {
            scope: self.thread_scope.clone(),
            thread_id: self.run_context.thread_id.clone(),
            turn_run_id: self.run_context.run_id,
        };
        if let Some(record) = self
            .thread_service
            .read_structured_finalization(read_request)
            .await
            .map_err(storage_error)?
        {
            // A successor lease may recover a record written immediately
            // before its predecessor lost ownership. The run ID, candidate,
            // contract name, and schema digest are the stable replay identity;
            // the owner fence remains evidence of which worker performed the
            // inference, not part of read-side adoption.
            if !record_matches_replay(&record, &candidate.content, &contract_name, &schema_digest) {
                return Err(host_error(
                    AgentLoopHostErrorKind::TranscriptWriteFailed,
                    "structured finalization replay does not match the current run attempt",
                ));
            }
            self.restore_usage(record.accounting.usage);
            return Ok(record.raw_json);
        }

        let system_prompt = finalization_system_prompt(&contract)?;
        let max_input_tokens = finalization_max_input_tokens(
            self.prompt_context_budget.context_limit_tokens,
            &system_prompt,
        );
        let context_messages = ironclaw_loop_host::load_canonical_system_inference_context(
            self.thread_service.as_ref(),
            &self.thread_scope,
            &self.run_context,
            self.max_context_messages,
            self.prompt_context_budget,
        )
        .await?;
        let response = self
            .inference
            .call_system_inference(SystemInferenceRequest {
                task_id: SystemInferenceTaskId::new(),
                identity: SystemInferenceIdentity {
                    task_kind: SystemTaskKind::StructuredOutputFinalization,
                    prompt_source: SystemPromptSource::Static {
                        prompt_id: "structured_output_finalization"
                            .to_string()
                            .try_into()
                            .map_err(|reason| {
                                host_error(AgentLoopHostErrorKind::Internal, reason)
                            })?,
                    },
                    system_prompt,
                },
                input_text: String::new(),
                context_messages,
                max_input_tokens,
                deadline_ms: FINALIZATION_DEADLINE_MS,
                output_contract: Some(contract.clone()),
            })
            .await
            .map_err(system_inference_error)?;
        // Capture provider usage before any subsequent validation, lease
        // fencing, or persistence step can fail. A failed finalization still
        // consumed this model work and must report it to the run exit.
        self.restore_usage(response.usage.map(to_storage_usage));
        // Do not publish a model result after ownership has been lost.  The
        // inference may finish after recovery reclaimed the lease.
        ironclaw_loop_host::ensure_run_lease_is_current(
            self.runtime.as_ref(),
            &self.run_context,
            self.lease_token,
        )
        .await?;
        let record = StructuredFinalizationRecord {
            scope: self.thread_scope.clone(),
            thread_id: self.run_context.thread_id.clone(),
            turn_id: self.run_context.turn_id,
            turn_run_id: self.run_context.run_id,
            contract_name: contract_name.clone(),
            schema_digest: schema_digest.clone(),
            candidate: candidate.content.clone(),
            raw_json: response.output_text,
            accounting: StructuredFinalizationAccounting {
                usage: response.usage.map(to_storage_usage),
                elapsed_ms: response.elapsed_ms,
                model_profile_id: Some(
                    self.run_context
                        .resolved_run_profile
                        .model_profile_id
                        .as_str()
                        .to_string(),
                ),
                provider_id: self
                    .run_context
                    .resolved_model_route
                    .as_ref()
                    .map(|route| route.provider_id().to_string()),
                model_id: self
                    .run_context
                    .resolved_model_route
                    .as_ref()
                    .map(|route| route.model_id().to_string()),
            },
            owner_fence: self.lease_token.as_uuid().to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let stored = match self
            .thread_service
            .put_structured_finalization(PutStructuredFinalizationRequest { record })
            .await
        {
            Ok(stored) => stored,
            Err(ironclaw_threads::SessionThreadError::StructuredFinalizationConflict {
                ..
            }) => {
                let recovered = self
                    .thread_service
                    .read_structured_finalization(ReadStructuredFinalizationRequest {
                        scope: self.thread_scope.clone(),
                        thread_id: self.run_context.thread_id.clone(),
                        turn_run_id: self.run_context.run_id,
                    })
                    .await
                    .map_err(storage_error)?;
                match recovered {
                    Some(record)
                        if record_matches_replay(
                            &record,
                            &candidate.content,
                            &contract_name,
                            &schema_digest,
                        ) =>
                    {
                        record
                    }
                    _ => {
                        return Err(host_error(
                            AgentLoopHostErrorKind::TranscriptWriteFailed,
                            "structured finalization conflict does not match the current run",
                        ));
                    }
                }
            }
            Err(error) => return Err(storage_error(error)),
        };
        // Close the check/write race fail-closed. A reclaimed worker cannot
        // publish this result, but a successor may recover the immutable record
        // by the stable replay identity checked above.
        ironclaw_loop_host::ensure_run_lease_is_current(
            self.runtime.as_ref(),
            &self.run_context,
            self.lease_token,
        )
        .await?;
        self.restore_usage(stored.accounting.usage);
        Ok(stored.raw_json)
    }

    fn supplemental_usage(&self) -> Option<LoopModelUsage> {
        *self
            .supplemental_usage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn restore_usage(&self, usage: Option<StructuredFinalizationUsage>) {
        let Some(usage) = usage else { return };
        let mut snapshot = self
            .supplemental_usage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // This host owns at most one logical finalizer call.  Re-entering the
        // idempotent transcript boundary must restore the same durable usage,
        // not add it a second time to the exit snapshot.
        *snapshot = Some(to_loop_usage(usage));
    }
}

fn finalization_max_input_tokens(context_limit_tokens: u64, system_prompt: &str) -> u64 {
    context_limit_tokens
        .saturating_add(ironclaw_loop_host::estimate_tokens_from_chars(system_prompt).as_u64())
}

#[async_trait]
impl<S> StructuredFinalizationPort for StructuredFinalizationCoordinator<S>
where
    S: SessionThreadService + ?Sized + Send + Sync,
{
    async fn finalize_terminal_reply(
        &self,
        message_ref: &LoopMessageRef,
    ) -> Result<(), AgentLoopHostError> {
        let raw_message_id = message_ref.as_str().strip_prefix("msg:").ok_or_else(|| {
            host_error(
                AgentLoopHostErrorKind::InvalidInvocation,
                "terminal reply reference is not a transcript message reference",
            )
        })?;
        let message_id = ThreadMessageId::parse(raw_message_id).map_err(|error| {
            tracing::debug!(%error, "terminal reply reference is not a valid transcript message id");
            host_error(
                AgentLoopHostErrorKind::InvalidInvocation,
                "terminal reply reference is not a valid transcript message id",
            )
        })?;
        let message = self
            .thread_service
            .read_thread_message(&self.thread_scope, &self.run_context.thread_id, message_id)
            .await
            .map_err(storage_error)?
            .ok_or_else(|| {
                host_error(
                    AgentLoopHostErrorKind::TranscriptWriteFailed,
                    "terminal assistant message is missing",
                )
            })?;
        let run_id = self.run_context.run_id;
        let run_id_text = run_id.to_string();
        if message.kind != MessageKind::Assistant
            || message.status != MessageStatus::Finalized
            || message.turn_run_id.as_deref() != Some(run_id_text.as_str())
        {
            return Err(host_error(
                AgentLoopHostErrorKind::TranscriptWriteFailed,
                "terminal reply reference is not the finalized assistant for this run",
            ));
        }
        let candidate = message.content.ok_or_else(|| {
            host_error(
                AgentLoopHostErrorKind::TranscriptWriteFailed,
                "terminal assistant message has no content",
            )
        })?;
        let raw_json = self
            .finalize_candidate(&AssistantReply { content: candidate })
            .await?;
        // The durable record and the transcript publication are separate
        // writes. Re-check the claimed lease immediately before the second
        // write so a reclaimed worker cannot publish after its finalization
        // evidence was adopted.
        ironclaw_loop_host::ensure_run_lease_is_current(
            self.runtime.as_ref(),
            &self.run_context,
            self.lease_token,
        )
        .await?;
        self.thread_service
            .publish_structured_finalization_message(PublishStructuredFinalizationMessageRequest {
                scope: self.thread_scope.clone(),
                thread_id: self.run_context.thread_id.clone(),
                message_id,
                turn_run_id: run_id,
                replacement: raw_json,
            })
            .await
            .map_err(storage_error)?;
        Ok(())
    }

    fn supplemental_model_usage(&self) -> Option<LoopModelUsage> {
        self.supplemental_usage()
    }
}

fn finalization_system_prompt(contract: &OutputContract) -> Result<String, AgentLoopHostError> {
    match contract {
        OutputContract::JsonSchema { schema, .. } => {
            let schema = serde_json::to_string(schema).map_err(|error| {
                tracing::debug!(%error, "structured finalization schema serialization failed");
                host_error(
                    AgentLoopHostErrorKind::Internal,
                    "schema serialization failed",
                )
            })?;
            Ok(format!(
                "{}\n\nDeclared output schema:\n{}",
                ironclaw_loop_host::STRUCTURED_OUTPUT_FINALIZATION_PROMPT.trim(),
                schema
            ))
        }
        OutputContract::JsonObject => Ok(format!(
            "{}\n\nDeclared output mode: return one valid JSON object. Output only the JSON object; do not emit Markdown fences, explanations, or additional text.",
            ironclaw_loop_host::STRUCTURED_OUTPUT_FINALIZATION_PROMPT.trim(),
        )),
        OutputContract::AssistantMessage => Err(host_error(
            AgentLoopHostErrorKind::InvalidInvocation,
            "structured finalization requires a structured output contract",
        )),
    }
}

fn contract_identity(contract: &OutputContract) -> Result<(String, String), AgentLoopHostError> {
    match contract {
        OutputContract::JsonSchema { name, schema } => {
            let schema = serde_json::to_vec(schema).map_err(|error| {
                tracing::debug!(%error, "structured finalization schema identity serialization failed");
                host_error(
                    AgentLoopHostErrorKind::Internal,
                    "schema serialization failed",
                )
            })?;
            Ok((name.clone(), blake3::hash(&schema).to_hex().to_string()))
        }
        OutputContract::JsonObject => Ok((
            "json_object".to_string(),
            blake3::hash(b"json_object").to_hex().to_string(),
        )),
        OutputContract::AssistantMessage => Err(host_error(
            AgentLoopHostErrorKind::InvalidInvocation,
            "structured finalization requires a structured output contract",
        )),
    }
}

fn record_matches_replay(
    record: &StructuredFinalizationRecord,
    candidate: &str,
    contract_name: &str,
    schema_digest: &str,
) -> bool {
    // The exact assistant row is either still carrying the ordinary candidate
    // or has already been CAS-replaced with this record's raw JSON. Both are
    // durable, run-scoped replay states; neither permits a new inference.
    (record.candidate == candidate || record.raw_json == candidate)
        && record.contract_name == contract_name
        && record.schema_digest == schema_digest
}

fn to_storage_usage(usage: LoopModelUsage) -> StructuredFinalizationUsage {
    StructuredFinalizationUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cache_read_input_tokens: usage.cache_read_input_tokens,
        cache_creation_input_tokens: usage.cache_creation_input_tokens,
    }
}

fn to_loop_usage(usage: StructuredFinalizationUsage) -> LoopModelUsage {
    LoopModelUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cache_read_input_tokens: usage.cache_read_input_tokens,
        cache_creation_input_tokens: usage.cache_creation_input_tokens,
    }
}

fn storage_error(error: ironclaw_threads::SessionThreadError) -> AgentLoopHostError {
    host_error(
        AgentLoopHostErrorKind::TranscriptWriteFailed,
        format!(
            "structured finalization persistence failed: {}",
            error.kind_name()
        ),
    )
}

fn system_inference_error(
    error: ironclaw_loop_contracts::SystemInferenceError,
) -> AgentLoopHostError {
    tracing::debug!(%error, "structured finalization inference failed");
    host_error(
        AgentLoopHostErrorKind::Unavailable,
        "structured finalization inference failed",
    )
}

fn host_error(kind: AgentLoopHostErrorKind, summary: impl Into<String>) -> AgentLoopHostError {
    AgentLoopHostError::new(kind, summary)
}

#[cfg(test)]
mod tests;
