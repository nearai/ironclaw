//! Python orchestrator — the self-modifiable execution loop.
//!
//! Replaces the Rust `ExecutionLoop::run()` with versioned Python code
//! executed via Monty. The orchestrator is the "glue layer" between the
//! LLM and tools — tool dispatch, output formatting, state management,
//! truncation — all in Python, patchable by the self-improvement Mission.
//!
//! Host functions exposed to the orchestrator Python:
//! - `__llm_complete__` — make an LLM call
//! - `__execute_code_step__` — run user CodeAct code in a nested Monty VM
//! - `__execute_action__` — execute a single tool action
//! - `__execute_actions_parallel__` — execute multiple tool actions concurrently
//! - `__check_signals__` — poll for stop/inject signals
//! - `__emit_event__` — broadcast a ThreadEvent
//! - `__save_checkpoint__` — persist thread state
//! - `__transition_to__` — change thread state (validated)
//! - `__retrieve_docs__` — query memory docs
//! - `__check_budget__` — remaining tokens/time/USD
//! - `__get_actions__` — available tool definitions

use std::sync::Arc;

use std::collections::HashMap;

use monty::{
    ExtFunctionResult, LimitedTracker, MontyObject, MontyRun, NameLookupResult, PrintWriter,
    ResourceLimits, RunProgress,
};
use tracing::debug;

use crate::capability::lease::LeaseManager;
use crate::capability::policy::PolicyEngine;
use crate::memory::RetrievalEngine;
use crate::runtime::messaging::{SignalReceiver, ThreadOutcome, ThreadSignal};
use crate::traits::effect::{EffectExecutor, ThreadExecutionContext};
use crate::traits::llm::{LlmBackend, LlmCallConfig};
use crate::traits::store::Store;
use crate::types::error::EngineError;
use crate::types::event::{EventKind, ThreadEvent, summarize_params};
use crate::types::message::ThreadMessage;
use crate::types::project::ProjectId;
use crate::types::shared_owner_id;
use crate::types::step::{StepId, TokenUsage};
use crate::types::thread::{ActiveSkillProvenance, Thread, ThreadState};

use super::scripting::{execute_code, json_to_monty, monty_to_json, monty_to_string};

/// The compiled-in default orchestrator (v0).
pub(crate) const DEFAULT_ORCHESTRATOR: &str = include_str!("../../orchestrator/default.py");

/// Well-known title for orchestrator code in the Store.
pub const ORCHESTRATOR_TITLE: &str = "orchestrator:main";

/// Well-known tag for orchestrator code docs.
pub const ORCHESTRATOR_TAG: &str = "orchestrator_code";

/// Result of running the orchestrator.
pub struct OrchestratorResult {
    /// The thread outcome parsed from the orchestrator's return value.
    pub outcome: ThreadOutcome,
    /// Total tokens used by LLM calls within the orchestrator.
    pub tokens_used: TokenUsage,
}

/// Extract source_channel from thread metadata (set by ConversationManager).
fn thread_source_channel(thread: &Thread) -> Option<String> {
    thread
        .metadata
        .get("source_channel")
        .and_then(|v| v.as_str())
        .map(String::from)
}

fn normalize_pause_outcome(
    thread: &mut Thread,
    outcome: &ThreadOutcome,
) -> Result<(), EngineError> {
    if matches!(outcome, ThreadOutcome::GatePaused { .. }) && thread.state != ThreadState::Waiting {
        thread.transition_to(
            ThreadState::Waiting,
            Some("waiting on external gate resolution".into()),
        )?;
    }
    Ok(())
}

/// Resource limits for the orchestrator VM.
fn orchestrator_limits() -> ResourceLimits {
    ResourceLimits::new()
        .max_duration(std::time::Duration::from_secs(300)) // 5 min (longer than user code)
        .max_allocations(5_000_000)
        .max_memory(128 * 1024 * 1024) // 128 MB
}

/// Maximum consecutive failures before auto-rollback.
const MAX_FAILURES_BEFORE_ROLLBACK: u64 = 3;

/// Well-known title for orchestrator failure tracking.
const FAILURE_TRACKER_TITLE: &str = "orchestrator:failures";

/// Load orchestrator code: runtime version from Store, or compiled-in default.
///
/// When `allow_self_modify` is false, always uses the compiled-in default
/// regardless of any runtime versions in the Store. This is the safe default
/// for production — runtime orchestrator patching is opt-in.
///
/// Checks the failure tracker — if the latest version has >= 3 consecutive
/// failures, falls back to the previous version (or compiled-in default).
pub async fn load_orchestrator(
    store: Option<&Arc<dyn Store>>,
    project_id: ProjectId,
    allow_self_modify: bool,
) -> (String, u64) {
    if !allow_self_modify {
        debug!("orchestrator self-modification disabled, using compiled-in default (v0)");
        return (DEFAULT_ORCHESTRATOR.to_string(), 0);
    }

    let Some(store) = store else {
        debug!("using compiled-in default orchestrator (v0, no store)");
        return (DEFAULT_ORCHESTRATOR.to_string(), 0);
    };

    let docs = match store.list_shared_memory_docs(project_id).await {
        Ok(d) => d,
        Err(_) => {
            debug!("using compiled-in default orchestrator (v0, store error)");
            return (DEFAULT_ORCHESTRATOR.to_string(), 0);
        }
    };

    load_orchestrator_from_docs(&docs, allow_self_modify)
}

/// Load orchestrator from pre-fetched system memory docs.
///
/// When the caller already has the `list_memory_docs` result, use this to
/// avoid a duplicate Store query. Returns `(code, version)`.
///
/// Respects `allow_self_modify` — when false, always returns the compiled-in
/// default. The caller in `loop_engine.rs` passes this from engine config.
pub fn load_orchestrator_from_docs(
    docs: &[crate::types::memory::MemoryDoc],
    allow_self_modify: bool,
) -> (String, u64) {
    if !allow_self_modify {
        return (DEFAULT_ORCHESTRATOR.to_string(), 0);
    }

    // Find all orchestrator versions, sorted by version number descending
    let mut versions: Vec<_> = docs
        .iter()
        .filter(|d| d.title == ORCHESTRATOR_TITLE && d.tags.contains(&ORCHESTRATOR_TAG.to_string()))
        .collect();
    versions.sort_by(|a, b| {
        let va = a
            .metadata
            .get("version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let vb = b
            .metadata
            .get("version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        vb.cmp(&va) // descending
    });

    if versions.is_empty() {
        debug!("using compiled-in default orchestrator (v0)");
        return (DEFAULT_ORCHESTRATOR.to_string(), 0);
    }

    // Check failure count for the latest version
    let failures = load_failure_count(docs);

    for doc in &versions {
        let version = doc
            .metadata
            .get("version")
            .and_then(|v| v.as_u64())
            .unwrap_or(1);

        // Skip versions with too many failures (only check the latest)
        if version
            == versions[0]
                .metadata
                .get("version")
                .and_then(|v| v.as_u64())
                .unwrap_or(1)
            && failures >= MAX_FAILURES_BEFORE_ROLLBACK
        {
            debug!(
                version,
                failures, "orchestrator version has too many failures, skipping"
            );
            continue;
        }

        debug!(version, "loaded runtime orchestrator");
        return (doc.content.clone(), version);
    }

    // All versions failed — fall back to compiled-in default
    debug!("all orchestrator versions failed, using compiled-in default (v0)");
    (DEFAULT_ORCHESTRATOR.to_string(), 0)
}

/// Record a failure for the current orchestrator version.
pub async fn record_orchestrator_failure(
    store: &Arc<dyn Store>,
    project_id: ProjectId,
    version: u64,
) {
    use crate::types::memory::{DocType, MemoryDoc};

    let docs = match store.list_shared_memory_docs(project_id).await {
        Ok(docs) => docs,
        Err(e) => {
            debug!("failed to list memory docs for failure tracker: {e}");
            return;
        }
    };
    let existing = docs.iter().find(|d| d.title == FAILURE_TRACKER_TITLE);

    let mut tracker = if let Some(doc) = existing {
        doc.clone()
    } else {
        MemoryDoc::new(
            project_id,
            shared_owner_id(),
            DocType::Note,
            FAILURE_TRACKER_TITLE,
            "",
        )
        .with_tags(vec!["orchestrator_meta".to_string()])
    };

    // Store failure count as JSON in content: {"version": N, "count": M}
    let current: serde_json::Value =
        serde_json::from_str(&tracker.content).unwrap_or(serde_json::json!({}));
    let current_version = current.get("version").and_then(|v| v.as_u64()).unwrap_or(0);
    let current_count = current.get("count").and_then(|v| v.as_u64()).unwrap_or(0);

    let new_count = if current_version == version {
        current_count + 1
    } else {
        1 // new version, reset count
    };

    tracker.content = serde_json::json!({
        "version": version,
        "count": new_count,
    })
    .to_string();
    tracker.updated_at = chrono::Utc::now();

    if let Err(e) = store.save_memory_doc(&tracker).await {
        debug!("failed to save orchestrator failure tracker: {e}");
    }

    debug!(version, count = new_count, "recorded orchestrator failure");
}

/// Reset the failure counter (called after successful execution).
pub async fn reset_orchestrator_failures(store: &Arc<dyn Store>, project_id: ProjectId) {
    let docs = store
        .list_shared_memory_docs(project_id)
        .await
        .unwrap_or_default();
    let existing = docs.iter().find(|d| d.title == FAILURE_TRACKER_TITLE);

    if let Some(doc) = existing {
        let mut tracker = doc.clone();
        tracker.content = serde_json::json!({"version": 0, "count": 0}).to_string();
        tracker.updated_at = chrono::Utc::now();
        let _ = store.save_memory_doc(&tracker).await;
    }
}

/// Load failure count for the latest orchestrator version.
fn load_failure_count(docs: &[crate::types::memory::MemoryDoc]) -> u64 {
    docs.iter()
        .find(|d| d.title == FAILURE_TRACKER_TITLE)
        .and_then(|d| serde_json::from_str::<serde_json::Value>(&d.content).ok())
        .and_then(|v| v.get("count").and_then(|c| c.as_u64()))
        .unwrap_or(0)
}

/// Execute the orchestrator Python code with host function dispatch.
///
/// This is the core function that replaces `ExecutionLoop::run()`'s inner loop.
/// The orchestrator Python calls host functions via Monty's suspension mechanism,
/// and this function handles each suspension by delegating to the appropriate
/// Rust implementation.
#[allow(clippy::too_many_arguments)]
pub async fn execute_orchestrator(
    code: &str,
    thread: &mut Thread,
    llm: &Arc<dyn LlmBackend>,
    effects: &Arc<dyn EffectExecutor>,
    leases: &Arc<LeaseManager>,
    policy: &Arc<PolicyEngine>,
    signal_rx: &mut SignalReceiver,
    event_tx: Option<&tokio::sync::broadcast::Sender<ThreadEvent>>,
    retrieval: Option<&RetrievalEngine>,
    store: Option<&Arc<dyn Store>>,
    persisted_state: &serde_json::Value,
) -> Result<OrchestratorResult, EngineError> {
    let mut total_tokens = TokenUsage::default();

    // Build context variables for the orchestrator
    let (input_names, input_values) = build_orchestrator_inputs(thread, persisted_state);

    // Parse and compile
    let runner = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        MontyRun::new(code.to_string(), "orchestrator.py", input_names)
    })) {
        Ok(Ok(runner)) => runner,
        Ok(Err(e)) => {
            return Err(EngineError::Effect {
                reason: format!("Orchestrator parse error: {e}"),
            });
        }
        Err(_) => {
            return Err(EngineError::Effect {
                reason: "Monty VM panicked during orchestrator parsing".into(),
            });
        }
    };

    // Start execution
    let mut stdout = String::new();
    let tracker = LimitedTracker::new(orchestrator_limits());

    let run_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        runner.start(input_values, tracker, PrintWriter::Collect(&mut stdout))
    }));

    let mut progress = match run_result {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            return Err(EngineError::Effect {
                reason: format!("Orchestrator runtime error: {e}"),
            });
        }
        Err(_) => {
            return Err(EngineError::Effect {
                reason: "Monty VM panicked during orchestrator start".into(),
            });
        }
    };

    // Drive the orchestrator dispatch loop
    let mut final_result: Option<serde_json::Value> = None;

    loop {
        match progress {
            RunProgress::Complete(obj) => {
                // Use FINAL result if set, otherwise fall back to VM return value
                let result = if let Some(ref fr) = final_result {
                    fr.clone()
                } else {
                    monty_to_json(&obj)
                };
                sync_runtime_state(thread, result.get("state"));
                let outcome = parse_outcome(&result);
                sync_visible_outcome(thread, &outcome);
                normalize_pause_outcome(thread, &outcome)?;
                return Ok(OrchestratorResult {
                    outcome,
                    tokens_used: total_tokens,
                });
            }

            RunProgress::FunctionCall(call) => {
                let action_name = call.function_name.clone();
                let args = &call.args;
                let kwargs = &call.kwargs;

                debug!(action = %action_name, "orchestrator: host function call");

                let ext_result = match action_name.as_str() {
                    // FINAL(result) — orchestrator returns its outcome
                    "FINAL" => {
                        let val = args.first().map(monty_to_json).unwrap_or_default();
                        final_result = Some(val);
                        ExtFunctionResult::Return(MontyObject::None)
                    }

                    // __llm_complete__(messages, actions, config)
                    "__llm_complete__" => {
                        handle_llm_complete(
                            args,
                            kwargs,
                            thread,
                            llm,
                            effects,
                            leases,
                            &mut total_tokens,
                        )
                        .await
                    }

                    // __execute_code_step__(code, state)
                    "__execute_code_step__" => {
                        handle_execute_code_step(
                            args, kwargs, thread, llm, effects, leases, policy, event_tx,
                        )
                        .await
                    }

                    // __execute_action__(name, params, call_id=...)
                    "__execute_action__" => {
                        handle_execute_action(
                            args, kwargs, thread, effects, leases, policy, event_tx,
                        )
                        .await
                    }

                    // __execute_actions_parallel__(calls)
                    "__execute_actions_parallel__" => {
                        handle_execute_actions_parallel(
                            args, thread, effects, leases, policy, event_tx,
                        )
                        .await
                    }

                    // __check_signals__()
                    "__check_signals__" => handle_check_signals(signal_rx, thread),

                    // __emit_event__(kind, **data)
                    "__emit_event__" => handle_emit_event(args, kwargs, thread, event_tx),

                    // __save_checkpoint__(state, counters)
                    "__save_checkpoint__" => handle_save_checkpoint(args, kwargs, thread),

                    // __transition_to__(state, reason)
                    "__transition_to__" => handle_transition_to(args, kwargs, thread),

                    // __retrieve_docs__(goal, max_docs)
                    "__retrieve_docs__" => {
                        handle_retrieve_docs(args, kwargs, thread, retrieval).await
                    }

                    // __check_budget__()"
                    "__check_budget__" => handle_check_budget(thread),

                    // __get_actions__()
                    "__get_actions__" => handle_get_actions(thread, effects, leases).await,

                    // __list_skills__(max_candidates, max_tokens)
                    "__list_skills__" => handle_list_skills(args, thread, store).await,

                    // __record_skill_usage__(doc_id, success)
                    "__record_skill_usage__" => handle_record_skill_usage(args, store).await,

                    // __set_active_skills__(skills)
                    "__set_active_skills__" => handle_set_active_skills(args, thread),

                    // Unknown — let Monty resolve it (user-defined functions, builtins)
                    other => ExtFunctionResult::NotFound(other.to_string()),
                };

                // Resume the orchestrator VM
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    call.resume(ext_result, PrintWriter::Collect(&mut stdout))
                })) {
                    Ok(Ok(p)) => progress = p,
                    Ok(Err(e)) => {
                        return Err(EngineError::Effect {
                            reason: format!("Orchestrator error after resume: {e}"),
                        });
                    }
                    Err(_) => {
                        return Err(EngineError::Effect {
                            reason: "Monty VM panicked during orchestrator resume".into(),
                        });
                    }
                }

                // If FINAL was called, the VM should complete on next iteration
                if final_result.is_some() {
                    continue;
                }
            }

            RunProgress::NameLookup(lookup) => {
                // Undefined variable — resume with NameError
                let name = lookup.name.clone();
                debug!(name = %name, "orchestrator: unresolved name");
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    lookup.resume(
                        NameLookupResult::Undefined,
                        PrintWriter::Collect(&mut stdout),
                    )
                })) {
                    Ok(Ok(p)) => progress = p,
                    Ok(Err(e)) => {
                        return Err(EngineError::Effect {
                            reason: format!("Orchestrator NameError '{name}': {e}"),
                        });
                    }
                    Err(_) => {
                        return Err(EngineError::Effect {
                            reason: format!("Monty panic on NameLookup '{name}'"),
                        });
                    }
                }
            }

            RunProgress::OsCall(_) => {
                return Err(EngineError::Effect {
                    reason: "Orchestrator attempted OS call (blocked)".into(),
                });
            }

            RunProgress::ResolveFutures(_) => {
                return Err(EngineError::Effect {
                    reason: "Orchestrator attempted async (not supported)".into(),
                });
            }
        }
    }
}

// ── Host function handlers ──────────────────────────────────

/// Handle `__llm_complete__(messages, actions, config)`.
///
/// Calls the LLM and returns the response as a dict:
/// `{type: "text"|"code"|"actions", content/code/calls: ..., usage: {...}}`
///
async fn handle_llm_complete(
    args: &[MontyObject],
    _kwargs: &[(MontyObject, MontyObject)],
    thread: &mut Thread,
    llm: &Arc<dyn LlmBackend>,
    effects: &Arc<dyn EffectExecutor>,
    leases: &Arc<LeaseManager>,
    total_tokens: &mut TokenUsage,
) -> ExtFunctionResult {
    use crate::types::step::LlmResponse;

    let explicit_messages = args.first().map(monty_to_json).filter(|v| !v.is_null());
    let explicit_config = args.get(2).map(monty_to_json).filter(|v| !v.is_null());
    let messages = explicit_messages
        .as_ref()
        .and_then(json_to_thread_messages)
        .unwrap_or_else(|| thread.messages.clone());

    let active_leases = leases.active_for_thread(thread.id).await;
    let actions = effects
        .available_actions(&active_leases)
        .await
        .unwrap_or_default();

    let config = LlmCallConfig {
        max_tokens: explicit_config
            .as_ref()
            .and_then(|cfg| cfg.get("max_tokens"))
            .and_then(|v| v.as_u64())
            .and_then(|v| u32::try_from(v).ok()),
        temperature: explicit_config
            .as_ref()
            .and_then(|cfg| cfg.get("temperature"))
            .and_then(|v| v.as_f64())
            .map(|v| v as f32),
        force_text: explicit_config
            .as_ref()
            .and_then(|cfg| cfg.get("force_text"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        depth: thread.config.depth,
        metadata: HashMap::new(),
    };

    match llm.complete(&messages, &actions, &config).await {
        Ok(output) => {
            total_tokens.input_tokens += output.usage.input_tokens;
            total_tokens.output_tokens += output.usage.output_tokens;
            total_tokens.cost_usd += output.usage.cost_usd;

            let usage = serde_json::json!({
                "input_tokens": output.usage.input_tokens,
                "output_tokens": output.usage.output_tokens,
                "cost_usd": output.usage.cost_usd,
            });

            let result = match output.response {
                LlmResponse::Text(text) => {
                    serde_json::json!({"type": "text", "content": text, "usage": usage})
                }
                LlmResponse::Code { code, .. } => {
                    serde_json::json!({"type": "code", "code": code, "usage": usage})
                }
                LlmResponse::ActionCalls { calls, content } => {
                    let calls_json: Vec<serde_json::Value> = calls
                        .iter()
                        .map(|c| {
                            serde_json::json!({
                                "name": c.action_name,
                                "call_id": c.id,
                                "params": c.parameters,
                            })
                        })
                        .collect();
                    serde_json::json!({
                        "type": "actions",
                        "content": content,
                        "calls": calls_json,
                        "usage": usage
                    })
                }
            };

            ExtFunctionResult::Return(json_to_monty(&result))
        }
        Err(e) => ExtFunctionResult::Error(monty::MontyException::new(
            monty::ExcType::RuntimeError,
            Some(format!("LLM call failed: {e}")),
        )),
    }
}

/// Handle `__execute_code_step__(code, state)`.
///
/// Runs user CodeAct code in a nested Monty VM with full tool dispatch.
/// Returns a dict with stdout, return_value, action_results, etc.
#[allow(clippy::too_many_arguments)]
async fn handle_execute_code_step(
    args: &[MontyObject],
    _kwargs: &[(MontyObject, MontyObject)],
    thread: &mut Thread,
    llm: &Arc<dyn LlmBackend>,
    effects: &Arc<dyn EffectExecutor>,
    leases: &Arc<LeaseManager>,
    policy: &Arc<PolicyEngine>,
    event_tx: Option<&tokio::sync::broadcast::Sender<ThreadEvent>>,
) -> ExtFunctionResult {
    let code = match args.first() {
        Some(obj) => monty_to_string(obj),
        None => {
            return ExtFunctionResult::Error(monty::MontyException::new(
                monty::ExcType::TypeError,
                Some("__execute_code_step__ requires a code string".into()),
            ));
        }
    };

    let state = args
        .get(1)
        .map(monty_to_json)
        .unwrap_or(serde_json::json!({}));

    let exec_ctx = ThreadExecutionContext {
        thread_id: thread.id,
        thread_type: thread.thread_type,
        project_id: thread.project_id,
        user_id: thread.user_id.clone(),
        step_id: StepId::new(),
        current_call_id: None,
        source_channel: thread_source_channel(thread),
    };

    // Run user code in a nested Monty VM (same pattern as rlm_query)
    match Box::pin(execute_code(
        &code,
        thread,
        llm,
        effects,
        leases,
        policy,
        &exec_ctx,
        &[],
        &state,
    ))
    .await
    {
        Ok(result) => {
            // Broadcast events from code execution to the thread and event channel.
            // Without this, ActionExecuted events from CodeAct tool calls are lost
            // and never appear in traces.
            for event_kind in &result.events {
                let event = ThreadEvent::new(thread.id, event_kind.clone());
                if let Some(tx) = event_tx {
                    let _ = tx.send(event.clone());
                }
                thread.events.push(event);
            }
            thread.updated_at = chrono::Utc::now();

            let action_results: Vec<serde_json::Value> = result
                .action_results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "action_name": r.action_name,
                        "output": r.output,
                        "is_error": r.is_error,
                        "duration_ms": r.duration.as_millis(),
                    })
                })
                .collect();

            let result_json = serde_json::json!({
                "return_value": result.return_value,
                "stdout": result.stdout,
                "action_results": action_results,
                "final_answer": result.final_answer,
                "had_error": result.had_error,
                "pending_gate": result.need_approval.as_ref().map(|na| {
                    match na {
                        ThreadOutcome::GatePaused { gate_name, action_name, call_id, parameters, resume_kind, resume_output } => serde_json::json!({
                            "gate_paused": true,
                            "gate_name": gate_name,
                            "action_name": action_name,
                            "call_id": call_id,
                            "parameters": parameters,
                            "resume_kind": serde_json::to_value(resume_kind).unwrap_or_default(),
                            "resume_output": resume_output,
                        }),
                        _ => serde_json::Value::Null,
                    }
                }),
            });

            ExtFunctionResult::Return(json_to_monty(&result_json))
        }
        Err(e) => ExtFunctionResult::Error(monty::MontyException::new(
            monty::ExcType::RuntimeError,
            Some(format!("Code execution failed: {e}")),
        )),
    }
}

/// Handle `__execute_action__(name, params, call_id=...)`.
///
/// Single source of truth for action execution. Performs:
/// 1. Lease lookup
/// 2. Policy check
/// 3. Lease consumption
/// 4. Action execution via EffectExecutor
/// 5. Event emission (ActionExecuted/ActionFailed)
///
/// Python owns the working transcript and decides how tool outputs are
/// represented in internal message history.
async fn handle_execute_action(
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
    thread: &mut Thread,
    effects: &Arc<dyn EffectExecutor>,
    leases: &Arc<LeaseManager>,
    policy: &Arc<PolicyEngine>,
    event_tx: Option<&tokio::sync::broadcast::Sender<ThreadEvent>>,
) -> ExtFunctionResult {
    let name = match extract_string_arg(args, kwargs, "name", 0) {
        Some(n) => n,
        None => {
            return ExtFunctionResult::Error(monty::MontyException::new(
                monty::ExcType::TypeError,
                Some("__execute_action__ requires a name argument".into()),
            ));
        }
    };

    let params = args
        .get(1)
        .map(monty_to_json)
        .unwrap_or(serde_json::json!({}));

    let call_id = extract_string_kwarg(kwargs, "call_id").unwrap_or_default();

    let exec_ctx = ThreadExecutionContext {
        thread_id: thread.id,
        thread_type: thread.thread_type,
        project_id: thread.project_id,
        user_id: thread.user_id.clone(),
        step_id: StepId::new(),
        current_call_id: Some(call_id.clone()),
        source_channel: thread_source_channel(thread),
    };

    // Helper: emit event only. The orchestrator owns transcript recording.
    let emit_and_record = |thread: &mut Thread,
                           event_tx: Option<&tokio::sync::broadcast::Sender<ThreadEvent>>,
                           event_kind: EventKind,
                           _call_id: &str,
                           _action_name: &str,
                           _output: &serde_json::Value| {
        let event = ThreadEvent::new(thread.id, event_kind);
        if let Some(tx) = event_tx {
            let _ = tx.send(event.clone());
        }
        thread.events.push(event);
        thread.updated_at = chrono::Utc::now();
    };

    // 1. Find lease for this action
    let lease = match leases.find_lease_for_action(thread.id, &name).await {
        Some(l) => l,
        None => {
            let error = format!("No lease for action '{name}'");
            let output = serde_json::json!({"error": &error});
            emit_and_record(
                thread,
                event_tx,
                EventKind::ActionFailed {
                    step_id: exec_ctx.step_id,
                    action_name: name.clone(),
                    call_id: call_id.clone(),
                    error,
                    params_summary: None,
                },
                &call_id,
                &name,
                &output,
            );
            let result = serde_json::json!({
                "output": output,
                "is_error": true,
            });
            return ExtFunctionResult::Return(json_to_monty(&result));
        }
    };

    // 2. Check policy
    let action_def = effects
        .available_actions(std::slice::from_ref(&lease))
        .await
        .ok()
        .and_then(|actions| actions.into_iter().find(|a| a.name == name));

    if let Some(ref ad) = action_def {
        match policy.evaluate(ad, &lease, &[]) {
            crate::capability::policy::PolicyDecision::Deny { reason } => {
                let output = serde_json::json!({"error": format!("Denied: {reason}")});
                emit_and_record(
                    thread,
                    event_tx,
                    EventKind::ActionFailed {
                        step_id: exec_ctx.step_id,
                        action_name: name.clone(),
                        call_id: call_id.clone(),
                        error: reason,
                        params_summary: None,
                    },
                    &call_id,
                    &name,
                    &output,
                );
                let result = serde_json::json!({
                    "output": output,
                    "is_error": true,
                });
                return ExtFunctionResult::Return(json_to_monty(&result));
            }
            crate::capability::policy::PolicyDecision::RequireApproval { .. } => {
                let output = serde_json::json!({"status": "gate_paused", "gate_name": "approval"});
                emit_and_record(
                    thread,
                    event_tx,
                    EventKind::ApprovalRequested {
                        action_name: name.clone(),
                        call_id: call_id.clone(),
                        parameters: Some(params.clone()),
                        description: None,
                        allow_always: None,
                        gate_name: None,
                        params_summary: summarize_params(&name, &params),
                    },
                    &call_id,
                    &name,
                    &output,
                );
                let result = serde_json::json!({
                    "gate_paused": true,
                    "gate_name": "approval",
                    "action_name": name,
                    "call_id": call_id,
                    "parameters": params,
                    "resume_kind": serde_json::to_value(crate::gate::ResumeKind::Approval {
                        allow_always: true,
                    })
                    .unwrap_or_default(),
                });
                return ExtFunctionResult::Return(json_to_monty(&result));
            }
            crate::capability::policy::PolicyDecision::Allow => {}
        }
    }

    // 3. Consume a lease use
    if let Err(e) = leases.consume_use(lease.id).await {
        debug!(error = %e, "lease consumption failed (non-fatal)");
    }

    // 4. Execute
    let ps = summarize_params(&name, &params);
    match effects
        .execute_action(&name, params, &lease, &exec_ctx)
        .await
    {
        Ok(r) => {
            emit_and_record(
                thread,
                event_tx,
                EventKind::ActionExecuted {
                    step_id: exec_ctx.step_id,
                    action_name: name.clone(),
                    call_id: call_id.clone(),
                    duration_ms: r.duration.as_millis() as u64,
                    params_summary: ps.clone(),
                },
                &call_id,
                &name,
                &r.output,
            );
            let result = serde_json::json!({
                "action_name": r.action_name,
                "output": r.output,
                "is_error": r.is_error,
                "duration_ms": r.duration.as_millis(),
            });
            ExtFunctionResult::Return(json_to_monty(&result))
        }
        Err(EngineError::GatePaused {
            gate_name,
            action_name: _,
            call_id: _,
            parameters,
            resume_kind,
            resume_output,
        }) => {
            let _ = leases.refund_use(lease.id).await;
            let output = serde_json::json!({"status": "gate_paused", "gate_name": gate_name});
            emit_and_record(
                thread,
                event_tx,
                EventKind::ApprovalRequested {
                    action_name: name.clone(),
                    call_id: call_id.clone(),
                    parameters: Some((*parameters).clone()),
                    description: None,
                    allow_always: match resume_kind.as_ref() {
                        crate::gate::ResumeKind::Approval { allow_always } => Some(*allow_always),
                        _ => None,
                    },
                    gate_name: Some(gate_name.clone()),
                    params_summary: summarize_params(&name, &parameters),
                },
                &call_id,
                &name,
                &output,
            );
            let result = serde_json::json!({
                "gate_paused": true,
                "gate_name": gate_name,
                "action_name": name,
                "call_id": call_id,
                "parameters": parameters,
                "resume_kind": serde_json::to_value(&*resume_kind).unwrap_or_default(),
                "resume_output": resume_output,
            });
            ExtFunctionResult::Return(json_to_monty(&result))
        }
        Err(e) => {
            let output = serde_json::json!({"error": e.to_string()});
            emit_and_record(
                thread,
                event_tx,
                EventKind::ActionFailed {
                    step_id: exec_ctx.step_id,
                    action_name: name.clone(),
                    call_id: call_id.clone(),
                    error: e.to_string(),
                    params_summary: ps,
                },
                &call_id,
                &name,
                &output,
            );
            let result = serde_json::json!({
                "output": output,
                "is_error": true,
            });
            ExtFunctionResult::Return(json_to_monty(&result))
        }
    }
}

/// Handle `__execute_actions_parallel__(calls)`.
///
/// Batch host function that receives a list of action calls and executes them
/// concurrently. Each call is a dict with `name`, `params`, and optionally `call_id`.
///
/// Returns a list of result dicts (one per call, in order). Each result has the
/// same shape as `__execute_action__` output, plus an optional gate pause payload.
///
/// Events are emitted in original call order after all parallel executions complete.
async fn handle_execute_actions_parallel(
    args: &[MontyObject],
    thread: &mut Thread,
    effects: &Arc<dyn EffectExecutor>,
    leases: &Arc<LeaseManager>,
    policy: &Arc<PolicyEngine>,
    event_tx: Option<&tokio::sync::broadcast::Sender<ThreadEvent>>,
) -> ExtFunctionResult {
    // Parse the calls list from the first argument (list of dicts)
    let calls_json = args
        .first()
        .map(monty_to_json)
        .unwrap_or(serde_json::json!([]));
    let calls_array = match calls_json.as_array() {
        Some(arr) => arr.clone(),
        None => {
            return ExtFunctionResult::Error(monty::MontyException::new(
                monty::ExcType::TypeError,
                Some("__execute_actions_parallel__ requires a list of call dicts".into()),
            ));
        }
    };

    if calls_array.is_empty() {
        return ExtFunctionResult::Return(json_to_monty(&serde_json::json!([])));
    }

    // Parse each call dict into (name, params, call_id)
    struct ParsedCall {
        name: String,
        params: serde_json::Value,
        call_id: String,
    }

    let mut parsed: Vec<ParsedCall> = Vec::with_capacity(calls_array.len());
    for c in &calls_array {
        let name = c
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let params = c.get("params").cloned().unwrap_or(serde_json::json!({}));
        let call_id = c
            .get("call_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        parsed.push(ParsedCall {
            name,
            params,
            call_id,
        });
    }

    let step_id = StepId::new();

    // ── Phase 1: Preflight (sequential) ─────────────────────────
    // Check leases and policies. Denied → error result. Approval → interrupt.

    enum PfOutcome {
        Runnable {
            lease: crate::types::capability::CapabilityLease,
        },
        Error {
            result_json: serde_json::Value,
            event: EventKind,
            output: serde_json::Value,
        },
    }

    let mut preflight: Vec<Option<PfOutcome>> = Vec::with_capacity(parsed.len());

    for pc in &parsed {
        // Find lease
        let lease = match leases.find_lease_for_action(thread.id, &pc.name).await {
            Some(l) => l,
            None => {
                let error = format!("No lease for action '{}'", pc.name);
                let output = serde_json::json!({"error": &error});
                let result_json = serde_json::json!({
                    "output": &output,
                    "is_error": true,
                });
                let event = EventKind::ActionFailed {
                    step_id,
                    action_name: pc.name.clone(),
                    call_id: pc.call_id.clone(),
                    error,
                    params_summary: None,
                };
                preflight.push(Some(PfOutcome::Error {
                    result_json,
                    event,
                    output,
                }));
                continue;
            }
        };

        // Check policy
        let action_def = effects
            .available_actions(std::slice::from_ref(&lease))
            .await
            .ok()
            .and_then(|actions| actions.into_iter().find(|a| a.name == pc.name));

        if let Some(ref ad) = action_def {
            match policy.evaluate(ad, &lease, &[]) {
                crate::capability::policy::PolicyDecision::Deny { reason } => {
                    let output = serde_json::json!({"error": format!("Denied: {reason}")});
                    let result_json = serde_json::json!({
                        "output": &output,
                        "is_error": true,
                    });
                    let event = EventKind::ActionFailed {
                        step_id,
                        action_name: pc.name.clone(),
                        call_id: pc.call_id.clone(),
                        error: reason,
                        params_summary: None,
                    };
                    preflight.push(Some(PfOutcome::Error {
                        result_json,
                        event,
                        output,
                    }));
                    continue;
                }
                crate::capability::policy::PolicyDecision::RequireApproval { .. } => {
                    // Emit events for earlier errors, then interrupt
                    let mut results_json = Vec::with_capacity(preflight.len() + 1);
                    for pf in preflight {
                        match pf {
                            Some(PfOutcome::Error {
                                result_json,
                                event,
                                output: _,
                            }) => {
                                let ev = ThreadEvent::new(thread.id, event);
                                if let Some(tx) = event_tx {
                                    let _ = tx.send(ev.clone());
                                }
                                thread.events.push(ev);
                                results_json.push(result_json);
                            }
                            Some(PfOutcome::Runnable { .. }) | None => {
                                results_json.push(serde_json::json!(null));
                            }
                        }
                    }
                    // Add the approval entry
                    let ev = ThreadEvent::new(
                        thread.id,
                        EventKind::ApprovalRequested {
                            action_name: pc.name.clone(),
                            call_id: pc.call_id.clone(),
                            parameters: Some(pc.params.clone()),
                            description: None,
                            allow_always: None,
                            gate_name: None,
                            params_summary: summarize_params(&pc.name, &pc.params),
                        },
                    );
                    if let Some(tx) = event_tx {
                        let _ = tx.send(ev.clone());
                    }
                    thread.events.push(ev);
                    thread.updated_at = chrono::Utc::now();

                    results_json.push(serde_json::json!({
                        "gate_paused": true,
                        "gate_name": "approval",
                        "action_name": &pc.name,
                        "call_id": &pc.call_id,
                        "parameters": &pc.params,
                        "resume_kind": serde_json::to_value(crate::gate::ResumeKind::Approval {
                            allow_always: true,
                        })
                        .unwrap_or_default(),
                    }));
                    return ExtFunctionResult::Return(json_to_monty(&serde_json::json!(
                        results_json
                    )));
                }
                crate::capability::policy::PolicyDecision::Allow => {}
            }
        }

        // Consume lease
        if let Err(e) = leases.consume_use(lease.id).await {
            debug!(error = %e, "lease consumption failed (non-fatal)");
        }

        preflight.push(Some(PfOutcome::Runnable { lease }));
    }

    // ── Phase 2: Execute in parallel ────────────────────────────

    // Slot array: index → execution result
    let mut slot_results: Vec<Option<serde_json::Value>> = vec![None; parsed.len()];
    let mut slot_events: Vec<Option<EventKind>> = vec![None; parsed.len()];
    let mut slot_outputs: Vec<Option<serde_json::Value>> = vec![None; parsed.len()];

    // Separate runnable from errors
    let mut runnable: Vec<(usize, crate::types::capability::CapabilityLease)> = Vec::new();
    for (idx, pf) in preflight.into_iter().enumerate() {
        match pf {
            Some(PfOutcome::Error {
                result_json,
                event,
                output,
            }) => {
                slot_results[idx] = Some(result_json);
                slot_events[idx] = Some(event);
                slot_outputs[idx] = Some(output);
            }
            Some(PfOutcome::Runnable { lease }) => {
                runnable.push((idx, lease));
            }
            None => {}
        }
    }

    if runnable.len() == 1 {
        // Single call: execute directly
        let (idx, lease) = runnable.into_iter().next().unwrap(); // safety: len()==1 checked above
        let pc = &parsed[idx];
        let exec_ctx = ThreadExecutionContext {
            thread_id: thread.id,
            thread_type: thread.thread_type,
            project_id: thread.project_id,
            user_id: thread.user_id.clone(),
            step_id,
            current_call_id: Some(pc.call_id.clone()),
            source_channel: None,
        };
        let ps = summarize_params(&pc.name, &pc.params);
        let (result_json, event, output) = execute_single_action(
            effects,
            &pc.name,
            pc.params.clone(),
            &pc.call_id,
            &lease,
            &exec_ctx,
            ps,
        )
        .await;
        if interrupted_result_needs_refund(&result_json) {
            let _ = leases.refund_use(lease.id).await;
        }
        slot_results[idx] = Some(result_json);
        slot_events[idx] = Some(event);
        slot_outputs[idx] = Some(output);
    } else if runnable.len() > 1 {
        // Multiple calls: execute in parallel via JoinSet
        let mut join_set = tokio::task::JoinSet::new();
        let effects = effects.clone();

        for (idx, lease) in runnable {
            let pc_name = parsed[idx].name.clone();
            let pc_params = parsed[idx].params.clone();
            let pc_call_id = parsed[idx].call_id.clone();
            let effects = effects.clone();
            let lease = lease.clone();
            let exec_ctx = ThreadExecutionContext {
                thread_id: thread.id,
                thread_type: thread.thread_type,
                project_id: thread.project_id,
                user_id: thread.user_id.clone(),
                step_id,
                current_call_id: Some(pc_call_id.clone()),
                source_channel: None,
            };
            let ps = summarize_params(&pc_name, &pc_params);

            join_set.spawn(async move {
                let (result_json, event, output) = execute_single_action(
                    &effects,
                    &pc_name,
                    pc_params,
                    &pc_call_id,
                    &lease,
                    &exec_ctx,
                    ps,
                )
                .await;
                (idx, lease.id, result_json, event, output)
            });
        }

        while let Some(join_result) = join_set.join_next().await {
            match join_result {
                Ok((idx, lease_id, result_json, event, output)) => {
                    if interrupted_result_needs_refund(&result_json) {
                        let _ = leases.refund_use(lease_id).await;
                    }
                    slot_results[idx] = Some(result_json);
                    slot_events[idx] = Some(event);
                    slot_outputs[idx] = Some(output);
                }
                Err(e) => {
                    debug!("parallel action execution task panicked: {e}");
                }
            }
        }
    }

    // ── Phase 3: Emit events in order ───────────────────────────

    let mut results_json = Vec::with_capacity(parsed.len());
    for idx in 0..parsed.len() {
        let result_json = slot_results[idx].take().unwrap_or(
            serde_json::json!({"is_error": true, "output": {"error": "execution slot empty"}}),
        );
        let _output = slot_outputs[idx]
            .take()
            .unwrap_or(serde_json::json!({"error": "no output"}));

        if let Some(event) = slot_events[idx].take() {
            let ev = ThreadEvent::new(thread.id, event);
            if let Some(tx) = event_tx {
                let _ = tx.send(ev.clone());
            }
            thread.events.push(ev);
        }

        results_json.push(result_json.clone());
    }

    thread.updated_at = chrono::Utc::now();
    ExtFunctionResult::Return(json_to_monty(&serde_json::json!(results_json)))
}

/// Execute a single action and return (result_json, event, output) for the
/// batch handler to record. Shared by both single-call and parallel paths.
async fn execute_single_action(
    effects: &Arc<dyn EffectExecutor>,
    name: &str,
    params: serde_json::Value,
    call_id: &str,
    lease: &crate::types::capability::CapabilityLease,
    exec_ctx: &ThreadExecutionContext,
    params_summary: Option<String>,
) -> (serde_json::Value, EventKind, serde_json::Value) {
    match effects.execute_action(name, params, lease, exec_ctx).await {
        Ok(r) => {
            let event = EventKind::ActionExecuted {
                step_id: exec_ctx.step_id,
                action_name: name.to_string(),
                call_id: call_id.to_string(),
                duration_ms: r.duration.as_millis() as u64,
                params_summary: params_summary.clone(),
            };
            let result_json = serde_json::json!({
                "action_name": r.action_name,
                "output": r.output,
                "is_error": r.is_error,
                "duration_ms": r.duration.as_millis(),
            });
            (result_json, event, r.output)
        }
        Err(EngineError::GatePaused {
            gate_name,
            action_name: _,
            call_id: _,
            parameters,
            resume_kind,
            resume_output,
        }) => {
            let output = serde_json::json!({"status": "gate_paused", "gate_name": &gate_name});
            let event = EventKind::ApprovalRequested {
                action_name: name.to_string(),
                call_id: call_id.to_string(),
                parameters: Some((*parameters).clone()),
                description: None,
                allow_always: match resume_kind.as_ref() {
                    crate::gate::ResumeKind::Approval { allow_always } => Some(*allow_always),
                    _ => None,
                },
                gate_name: Some(gate_name.clone()),
                params_summary: summarize_params(name, &parameters),
            };
            let result_json = serde_json::json!({
                "gate_paused": true,
                "gate_name": gate_name,
                "action_name": name,
                "call_id": call_id,
                "parameters": parameters,
                "resume_kind": serde_json::to_value(&*resume_kind).unwrap_or_default(),
                "resume_output": resume_output,
            });
            (result_json, event, output)
        }
        Err(e) => {
            let output = serde_json::json!({"error": e.to_string()});
            let event = EventKind::ActionFailed {
                step_id: exec_ctx.step_id,
                action_name: name.to_string(),
                call_id: call_id.to_string(),
                error: e.to_string(),
                params_summary,
            };
            let result_json = serde_json::json!({
                "output": &output,
                "is_error": true,
            });
            (result_json, event, output)
        }
    }
}

fn interrupted_result_needs_refund(result: &serde_json::Value) -> bool {
    result.get("gate_paused").and_then(|v| v.as_bool()) == Some(true)
}

/// Handle `__check_signals__()`.
fn handle_check_signals(signal_rx: &mut SignalReceiver, thread: &mut Thread) -> ExtFunctionResult {
    match signal_rx.try_recv() {
        Ok(ThreadSignal::Stop) | Ok(ThreadSignal::Suspend) => {
            ExtFunctionResult::Return(MontyObject::String("stop".into()))
        }
        Ok(ThreadSignal::InjectMessage(msg)) => {
            thread.add_message(msg.clone());
            let result = serde_json::json!({"inject": msg.content});
            ExtFunctionResult::Return(json_to_monty(&result))
        }
        Ok(ThreadSignal::Resume) | Ok(ThreadSignal::ChildCompleted { .. }) => {
            ExtFunctionResult::Return(MontyObject::None)
        }
        Err(_) => ExtFunctionResult::Return(MontyObject::None),
    }
}

/// Handle `__emit_event__(kind, **data)`.
fn handle_emit_event(
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
    thread: &mut Thread,
    event_tx: Option<&tokio::sync::broadcast::Sender<ThreadEvent>>,
) -> ExtFunctionResult {
    let kind_str = args.first().map(monty_to_string).unwrap_or_default();

    let kind = match kind_str.as_str() {
        "step_started" => {
            let _step = extract_u64_kwarg(kwargs, "step").unwrap_or(0);
            EventKind::StepStarted {
                step_id: StepId::new(),
            }
        }
        "step_completed" => {
            let input = extract_u64_kwarg(kwargs, "input_tokens").unwrap_or(0);
            let output = extract_u64_kwarg(kwargs, "output_tokens").unwrap_or(0);
            // Increment step count (mirrors the old Rust loop's step_count += 1)
            thread.step_count += 1;
            // Track token usage
            thread.total_tokens_used += input + output;
            EventKind::StepCompleted {
                step_id: StepId::new(),
                tokens: TokenUsage {
                    input_tokens: input,
                    output_tokens: output,
                    ..Default::default()
                },
            }
        }
        "action_executed" => {
            let action_name = extract_string_kwarg(kwargs, "action_name").unwrap_or_default();
            let call_id = extract_string_kwarg(kwargs, "call_id").unwrap_or_default();
            EventKind::ActionExecuted {
                step_id: StepId::new(),
                action_name,
                call_id,
                duration_ms: 0,
                params_summary: None,
            }
        }
        "action_failed" => {
            let action_name = extract_string_kwarg(kwargs, "action_name").unwrap_or_default();
            let call_id = extract_string_kwarg(kwargs, "call_id").unwrap_or_default();
            let error = extract_string_kwarg(kwargs, "error").unwrap_or_default();
            EventKind::ActionFailed {
                step_id: StepId::new(),
                action_name,
                call_id,
                error,
                params_summary: None,
            }
        }
        "skill_activated" => {
            let names_str = extract_string_kwarg(kwargs, "skill_names").unwrap_or_default();
            let skill_names: Vec<String> = names_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            EventKind::SkillActivated { skill_names }
        }
        _ => {
            debug!(kind = %kind_str, "orchestrator: unknown event kind, skipping");
            return ExtFunctionResult::Return(MontyObject::None);
        }
    };

    let event = ThreadEvent::new(thread.id, kind);
    if let Some(tx) = event_tx {
        let _ = tx.send(event.clone());
    }
    thread.events.push(event);
    thread.updated_at = chrono::Utc::now();

    ExtFunctionResult::Return(MontyObject::None)
}

/// Handle `__save_checkpoint__(state, counters)`.
fn handle_save_checkpoint(
    args: &[MontyObject],
    _kwargs: &[(MontyObject, MontyObject)],
    thread: &mut Thread,
) -> ExtFunctionResult {
    let state = args
        .first()
        .map(monty_to_json)
        .unwrap_or(serde_json::json!({}));
    let counters = args
        .get(1)
        .map(monty_to_json)
        .unwrap_or(serde_json::json!({}));

    sync_runtime_state(thread, Some(&state));

    if let Some(metadata) = thread.metadata.as_object_mut() {
        metadata.insert(
            "runtime_checkpoint".into(),
            serde_json::json!({
                "persisted_state": state,
                "nudge_count": counters.get("nudge_count").and_then(|v| v.as_u64()).unwrap_or(0),
                "consecutive_errors": counters.get("consecutive_errors").and_then(|v| v.as_u64()).unwrap_or(0),
                "compaction_count": counters.get("compaction_count").and_then(|v| v.as_u64()).unwrap_or(0),
            }),
        );
    }
    thread.updated_at = chrono::Utc::now();

    ExtFunctionResult::Return(MontyObject::None)
}

/// Handle `__transition_to__(state, reason)`.
fn handle_transition_to(
    args: &[MontyObject],
    _kwargs: &[(MontyObject, MontyObject)],
    thread: &mut Thread,
) -> ExtFunctionResult {
    let state_str = args.first().map(monty_to_string).unwrap_or_default();
    let reason = args.get(1).map(monty_to_string);

    let target = match state_str.as_str() {
        "running" => crate::types::thread::ThreadState::Running,
        "completed" => crate::types::thread::ThreadState::Completed,
        "failed" => crate::types::thread::ThreadState::Failed,
        "waiting" => crate::types::thread::ThreadState::Waiting,
        "suspended" => crate::types::thread::ThreadState::Suspended,
        other => {
            return ExtFunctionResult::Error(monty::MontyException::new(
                monty::ExcType::ValueError,
                Some(format!("Unknown thread state: {other}")),
            ));
        }
    };

    match thread.transition_to(target, reason) {
        Ok(()) => ExtFunctionResult::Return(MontyObject::None),
        Err(e) => ExtFunctionResult::Error(monty::MontyException::new(
            monty::ExcType::RuntimeError,
            Some(format!("State transition failed: {e}")),
        )),
    }
}

/// Handle `__retrieve_docs__(goal, max_docs)`.
async fn handle_retrieve_docs(
    args: &[MontyObject],
    _kwargs: &[(MontyObject, MontyObject)],
    thread: &Thread,
    retrieval: Option<&RetrievalEngine>,
) -> ExtFunctionResult {
    let retrieval = match retrieval {
        Some(r) => r,
        None => return ExtFunctionResult::Return(json_to_monty(&serde_json::json!([]))),
    };

    let goal = args.first().map(monty_to_string).unwrap_or_default();
    let max_docs = args
        .get(1)
        .and_then(|v| match v {
            MontyObject::Int(i) => Some(*i as usize),
            _ => None,
        })
        .unwrap_or(5);

    match retrieval
        .retrieve_context(thread.project_id, &thread.user_id, &goal, max_docs)
        .await
    {
        Ok(docs) => {
            let docs_json: Vec<serde_json::Value> = docs
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "type": format!("{:?}", d.doc_type),
                        "title": d.title,
                        "content": d.content,
                    })
                })
                .collect();
            ExtFunctionResult::Return(json_to_monty(&serde_json::json!(docs_json)))
        }
        Err(e) => {
            debug!("retrieve_docs failed: {e}");
            ExtFunctionResult::Return(json_to_monty(&serde_json::json!([])))
        }
    }
}

/// Handle `__check_budget__()`.
fn handle_check_budget(thread: &Thread) -> ExtFunctionResult {
    let tokens_remaining = thread
        .config
        .max_tokens_total
        .map(|max| max.saturating_sub(thread.total_tokens_used))
        .unwrap_or(u64::MAX);

    let time_remaining_ms = thread
        .config
        .max_duration
        .map(|dur| {
            let elapsed = chrono::Utc::now()
                .signed_duration_since(thread.created_at)
                .num_milliseconds()
                .max(0) as u64;
            dur.as_millis() as u64 - elapsed.min(dur.as_millis() as u64)
        })
        .unwrap_or(u64::MAX);

    let usd_remaining = thread
        .config
        .max_budget_usd
        .map(|max| (max - thread.total_cost_usd).max(0.0));

    let result = serde_json::json!({
        "tokens_remaining": tokens_remaining,
        "time_remaining_ms": time_remaining_ms,
        "usd_remaining": usd_remaining,
    });

    ExtFunctionResult::Return(json_to_monty(&result))
}

/// Handle `__get_actions__()`.
async fn handle_get_actions(
    thread: &Thread,
    effects: &Arc<dyn EffectExecutor>,
    leases: &Arc<LeaseManager>,
) -> ExtFunctionResult {
    let active_leases = leases.active_for_thread(thread.id).await;
    match effects.available_actions(&active_leases).await {
        Ok(actions) => {
            let actions_json: Vec<serde_json::Value> = actions
                .iter()
                .map(|a| {
                    serde_json::json!({
                        "name": a.name,
                        "description": a.description,
                        "params": a.parameters_schema,
                    })
                })
                .collect();
            ExtFunctionResult::Return(json_to_monty(&serde_json::json!(actions_json)))
        }
        Err(e) => {
            debug!("get_actions failed: {e}");
            ExtFunctionResult::Return(json_to_monty(&serde_json::json!([])))
        }
    }
}

/// Handle `__list_skills__()`.
///
/// Loads all `DocType::Skill` MemoryDocs from the project and returns them
/// as a list of Python dicts. The Python orchestrator handles scoring,
/// selection, and injection — Rust just provides data access.
async fn handle_list_skills(
    _args: &[MontyObject],
    thread: &Thread,
    store: Option<&Arc<dyn Store>>,
) -> ExtFunctionResult {
    let Some(store) = store else {
        return ExtFunctionResult::Return(json_to_monty(&serde_json::json!([])));
    };

    // Use shared listing: user's own skills + system/admin-installed skills.
    let docs = match store
        .list_memory_docs_with_shared(thread.project_id, &thread.user_id)
        .await
    {
        Ok(docs) => docs,
        Err(e) => {
            debug!("__list_skills__: failed to load docs: {e}");
            return ExtFunctionResult::Return(json_to_monty(&serde_json::json!([])));
        }
    };

    let skills: Vec<serde_json::Value> = docs
        .into_iter()
        .filter(|d| d.doc_type == crate::types::memory::DocType::Skill)
        .map(|d| {
            serde_json::json!({
                "doc_id": d.id.0.to_string(),
                "title": d.title,
                "content": d.content,
                "metadata": d.metadata,
            })
        })
        .collect();

    ExtFunctionResult::Return(json_to_monty(&serde_json::json!(skills)))
}

/// Handle `__record_skill_usage__(doc_id, success)`.
///
/// Records that a skill was used in this thread. Called by the Python
/// orchestrator after skill-assisted execution completes.
async fn handle_record_skill_usage(
    args: &[MontyObject],
    store: Option<&Arc<dyn Store>>,
) -> ExtFunctionResult {
    let Some(store) = store else {
        return ExtFunctionResult::Return(MontyObject::None);
    };

    let doc_id_str = args.first().map(monty_to_string).unwrap_or_default();
    let success = args
        .get(1)
        .map(|o| matches!(o, MontyObject::Bool(true)))
        .unwrap_or(false);

    let Ok(uuid) = uuid::Uuid::parse_str(&doc_id_str) else {
        debug!("__record_skill_usage__: invalid doc_id: {doc_id_str}");
        return ExtFunctionResult::Return(MontyObject::None);
    };

    let tracker = crate::memory::SkillTracker::new(Arc::clone(store));
    if let Err(e) = tracker
        .record_usage(crate::types::memory::DocId(uuid), success)
        .await
    {
        debug!("__record_skill_usage__: failed: {e}");
    }

    ExtFunctionResult::Return(MontyObject::None)
}

/// Handle `__set_active_skills__(skills)`.
///
/// Persists the selected skill provenance onto the thread so post-run learning
/// flows can reason about the exact skill versions and snippets that were active.
fn handle_set_active_skills(args: &[MontyObject], thread: &mut Thread) -> ExtFunctionResult {
    let skills_json = args
        .first()
        .map(monty_to_json)
        .unwrap_or_else(|| serde_json::json!([]));

    let skills = match serde_json::from_value::<Vec<ActiveSkillProvenance>>(skills_json) {
        Ok(skills) => skills,
        Err(e) => {
            debug!("__set_active_skills__: invalid payload: {e}");
            return ExtFunctionResult::Return(MontyObject::None);
        }
    };

    if let Err(e) = thread.set_active_skills(&skills) {
        debug!("__set_active_skills__: failed to persist active skills: {e}");
    }

    ExtFunctionResult::Return(MontyObject::None)
}

// ── Helpers ─────────────────────────────────────────────────

/// Build the context variables injected into the orchestrator Python.
fn build_orchestrator_inputs(
    thread: &Thread,
    persisted_state: &serde_json::Value,
) -> (Vec<String>, Vec<MontyObject>) {
    let names = vec![
        "context".into(),
        "goal".into(),
        "actions".into(),
        "state".into(),
        "config".into(),
    ];

    // Build orchestrator bootstrap context. Prefer the internal execution
    // transcript when present, otherwise fall back to the user-visible transcript.
    let bootstrap_messages = if thread.internal_messages.is_empty() {
        &thread.messages
    } else {
        &thread.internal_messages
    };
    let context: Vec<serde_json::Value> = bootstrap_messages
        .iter()
        .map(|m| {
            serde_json::json!({
                "role": format!("{:?}", m.role),
                "content": m.content,
                "action_name": m.action_name,
                "action_call_id": m.action_call_id,
                "action_calls": m.action_calls,
            })
        })
        .collect();

    // Build config
    let config = serde_json::json!({
        "max_iterations": thread.config.max_iterations,
        "max_tool_intent_nudges": thread.config.max_tool_intent_nudges,
        "enable_tool_intent_nudge": thread.config.enable_tool_intent_nudge,
        "max_consecutive_errors": thread.config.max_consecutive_errors,
        "max_tokens_total": thread.config.max_tokens_total,
        "max_budget_usd": thread.config.max_budget_usd,
        "model_context_limit": thread.config.model_context_limit,
        "enable_compaction": thread.config.enable_compaction,
        "compaction_threshold": thread.config.compaction_threshold,
        "depth": thread.config.depth,
        "max_depth": thread.config.max_depth,
        "step_count": thread.step_count,
    });

    let values = vec![
        json_to_monty(&serde_json::json!(context)),
        MontyObject::String(thread.goal.clone()),
        json_to_monty(&serde_json::json!([])), // actions loaded dynamically via __get_actions__
        json_to_monty(persisted_state),
        json_to_monty(&config),
    ];

    (names, values)
}

fn json_to_thread_messages(value: &serde_json::Value) -> Option<Vec<ThreadMessage>> {
    let arr = value.as_array()?;
    let mut messages = Vec::with_capacity(arr.len());

    for item in arr {
        let role = item.get("role").and_then(|v| v.as_str()).unwrap_or("User");
        let content = item
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let action_calls = item
            .get("action_calls")
            .and_then(|v| serde_json::from_value(v.clone()).ok());

        let message = match role {
            "System" | "system" => ThreadMessage::system(content),
            "Assistant" | "assistant" => {
                if let Some(calls) = action_calls {
                    ThreadMessage::assistant_with_actions(Some(content.to_string()), calls)
                } else {
                    ThreadMessage::assistant(content)
                }
            }
            "ActionResult" | "action_result" => ThreadMessage::action_result(
                item.get("action_call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default(),
                item.get("action_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default(),
                content,
            ),
            _ => ThreadMessage::user(content),
        };
        messages.push(message);
    }

    Some(messages)
}

fn sync_runtime_state(thread: &mut Thread, state: Option<&serde_json::Value>) {
    let Some(state) = state else {
        return;
    };
    if let Some(messages) = state
        .get("working_messages")
        .and_then(json_to_thread_messages)
    {
        thread.internal_messages = messages;
        thread.updated_at = chrono::Utc::now();
    }
}

fn sync_visible_outcome(thread: &mut Thread, outcome: &ThreadOutcome) {
    if let ThreadOutcome::Completed {
        response: Some(response),
    } = outcome
    {
        let already_present = thread
            .messages
            .last()
            .map(|msg| {
                msg.role == crate::types::message::MessageRole::Assistant
                    && msg.content == *response
            })
            .unwrap_or(false);
        if !already_present {
            thread.add_message(ThreadMessage::assistant(response));
        }
    }
}

/// Parse the orchestrator's return value into a ThreadOutcome.
fn parse_outcome(result: &serde_json::Value) -> ThreadOutcome {
    let outcome = result
        .get("outcome")
        .and_then(|v| v.as_str())
        .unwrap_or("completed");

    match outcome {
        "completed" => ThreadOutcome::Completed {
            response: result
                .get("response")
                .and_then(|v| v.as_str())
                .map(String::from),
        },
        "stopped" => ThreadOutcome::Stopped,
        "max_iterations" => ThreadOutcome::MaxIterations,
        "failed" => ThreadOutcome::Failed {
            error: result
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error")
                .to_string(),
        },
        "gate_paused" => {
            let resume_kind_value = result
                .get("resume_kind")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            let resume_kind = serde_json::from_value(resume_kind_value).unwrap_or(
                crate::gate::ResumeKind::Approval {
                    allow_always: false,
                },
            );
            ThreadOutcome::GatePaused {
                gate_name: result
                    .get("gate_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                action_name: result
                    .get("action_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                call_id: result
                    .get("call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                parameters: result
                    .get("parameters")
                    .cloned()
                    .unwrap_or(serde_json::json!({})),
                resume_kind,
                resume_output: result.get("resume_output").cloned(),
            }
        }
        _ => ThreadOutcome::Completed { response: None },
    }
}

fn extract_string_arg(
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
    name: &str,
    position: usize,
) -> Option<String> {
    for (k, v) in kwargs {
        if let MontyObject::String(key) = k
            && key == name
        {
            return Some(monty_to_string(v));
        }
    }
    args.get(position).map(monty_to_string)
}

fn extract_string_kwarg(kwargs: &[(MontyObject, MontyObject)], name: &str) -> Option<String> {
    for (k, v) in kwargs {
        if let MontyObject::String(key) = k
            && key == name
        {
            return Some(monty_to_string(v));
        }
    }
    None
}

fn extract_u64_kwarg(kwargs: &[(MontyObject, MontyObject)], name: &str) -> Option<u64> {
    for (k, v) in kwargs {
        if let MontyObject::String(key) = k
            && key == name
            && let MontyObject::Int(i) = v
        {
            return Some(*i as u64);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::memory::{DocType, MemoryDoc};
    use crate::types::project::ProjectId;

    // ── Python helper unit tests via Monty ──────────────────────
    //
    // Extracts the helper functions from the default orchestrator and
    // evaluates `signals_tool_intent(text)` directly, mirroring the V1
    // Rust unit test suite in src/llm/reasoning.rs.

    /// Run a Python expression that returns a bool by prepending the
    /// orchestrator helper definitions and wrapping in `FINAL(expr)`.
    fn eval_python_bool(expr: &str) -> bool {
        // Extract only the helper functions (everything before run_loop)
        let helpers_end = DEFAULT_ORCHESTRATOR
            .find("\ndef run_loop(")
            .unwrap_or(DEFAULT_ORCHESTRATOR.len());
        let helpers = &DEFAULT_ORCHESTRATOR[..helpers_end];

        let code = format!("{helpers}\nFINAL({expr})");

        let runner = MontyRun::new(code.to_string(), "test.py", vec![])
            .expect("Failed to parse orchestrator helpers");
        let mut stdout = String::new();
        let tracker = LimitedTracker::new(ResourceLimits::new().max_allocations(500_000));

        let mut progress = runner
            .start(vec![], tracker, PrintWriter::Collect(&mut stdout))
            .expect("Failed to start orchestrator test");

        // Drive the VM — handle the FINAL() host call
        loop {
            match progress {
                RunProgress::Complete(obj) => {
                    return match obj {
                        MontyObject::Bool(v) => v,
                        other => panic!("Expected bool, got: {other:?}"),
                    };
                }
                RunProgress::FunctionCall(call) => {
                    if call.function_name == "FINAL" {
                        let val = call.args.first().cloned().unwrap_or(MontyObject::None);
                        // Resume and discard — we already have the value
                        let _ = call.resume(
                            ExtFunctionResult::Return(MontyObject::None),
                            PrintWriter::Collect(&mut stdout),
                        );
                        return match val {
                            MontyObject::Bool(v) => v,
                            other => panic!("FINAL() received non-bool: {other:?}"),
                        };
                    }
                    // Unknown host function — return None and continue
                    progress = call
                        .resume(
                            ExtFunctionResult::Return(MontyObject::None),
                            PrintWriter::Collect(&mut stdout),
                        )
                        .expect("resume failed");
                }
                RunProgress::NameLookup(lookup) => {
                    progress = lookup
                        .resume(
                            NameLookupResult::Undefined,
                            PrintWriter::Collect(&mut stdout),
                        )
                        .expect("name lookup resume failed");
                }
                _ => panic!("Unexpected RunProgress variant in test"),
            }
        }
    }

    // ── True positives (should trigger nudge) ───────────────────

    #[test]
    fn signals_tool_intent_true_positives() {
        assert!(eval_python_bool(
            r#"signals_tool_intent("Let me search for that file.")"#
        ));
        assert!(eval_python_bool(
            r#"signals_tool_intent("I'll fetch the data now.")"#
        ));
        assert!(eval_python_bool(
            r#"signals_tool_intent("I'm going to check the logs.")"#
        ));
        assert!(eval_python_bool(
            r#"signals_tool_intent("Let me add it now.")"#
        ));
        assert!(eval_python_bool(
            r#"signals_tool_intent("I will run the tests to verify.")"#
        ));
        assert!(eval_python_bool(
            r#"signals_tool_intent("I'll look up the documentation.")"#
        ));
        assert!(eval_python_bool(
            r#"signals_tool_intent("Let me read the file contents.")"#
        ));
        assert!(eval_python_bool(
            r#"signals_tool_intent("I'm going to execute the command.")"#
        ));
    }

    // ── True negatives: conversational phrases ──────────────────

    #[test]
    fn signals_tool_intent_true_negatives_conversational() {
        assert!(!eval_python_bool(
            r#"signals_tool_intent("Let me explain how this works.")"#
        ));
        assert!(!eval_python_bool(
            r#"signals_tool_intent("Let me know if you need anything.")"#
        ));
        assert!(!eval_python_bool(
            r#"signals_tool_intent("Let me think about this.")"#
        ));
        assert!(!eval_python_bool(
            r#"signals_tool_intent("Let me summarize the findings.")"#
        ));
        assert!(!eval_python_bool(
            r#"signals_tool_intent("Let me clarify what I mean.")"#
        ));
    }

    // ── Exclusion takes precedence ──────────────────────────────

    #[test]
    fn signals_tool_intent_exclusion_takes_precedence() {
        assert!(!eval_python_bool(
            r#"signals_tool_intent("Let me explain the approach, then I'll search for the file.")"#
        ));
    }

    // ── Code blocks are stripped ────────────────────────────────

    #[test]
    fn signals_tool_intent_ignores_code_blocks() {
        assert!(!eval_python_bool(
            r#"signals_tool_intent("Here's the code:\n\n```\nfn main() {\n    println!(\"Let me search the database\");\n}\n```")"#
        ));
    }

    #[test]
    fn signals_tool_intent_ignores_indented_code() {
        assert!(!eval_python_bool(
            r#"signals_tool_intent("Here's the code:\n\n    println!(\"I'll fetch the data\");\n\nThat's it.")"#
        ));
    }

    // ── Plain informational text ────────────────────────────────

    #[test]
    fn signals_tool_intent_ignores_plain_text() {
        assert!(!eval_python_bool(
            r#"signals_tool_intent("The task is complete.")"#
        ));
        assert!(!eval_python_bool(
            r#"signals_tool_intent("Here are the results you asked for.")"#
        ));
        assert!(!eval_python_bool(
            r#"signals_tool_intent("I found 3 matching files.")"#
        ));
    }

    // ── Quoted strings are stripped ─────────────────────────────

    #[test]
    fn signals_tool_intent_ignores_quoted_strings() {
        assert!(!eval_python_bool(
            r#"signals_tool_intent("The button says \"Let me search the database\" to the user.")"#
        ));
        // But unquoted intent should still trigger
        assert!(eval_python_bool(
            r#"signals_tool_intent("I'll fetch the results for you.")"#
        ));
    }

    // ── Shadowed prefix (exclusion cancels all) ─────────────────

    #[test]
    fn signals_tool_intent_shadowed_prefix() {
        // "let me think" is an exclusion → entire text returns false
        assert!(!eval_python_bool(
            r#"signals_tool_intent("Sure, let me think about it. Actually, let me search for the file.")"#
        ));
    }

    // ── Regression: trace false positive (news content) ─────────

    #[test]
    fn signals_tool_intent_no_false_positive_news_content() {
        // "I can" + "call" in news content triggered false positive in old code
        let news_response = concat!(
            "The latest headlines suggest this is a fast-moving war.\n",
            "- Reuters: Iran is calling US peace proposals unrealistic.\n",
            "If you want, I can do one of these next:\n",
            "1. give you a 5-bullet update\n",
            "2. focus just on military developments",
        );
        assert!(!eval_python_bool(&format!(
            "signals_tool_intent({news_response:?})"
        )));
    }

    #[test]
    fn signals_tool_intent_no_false_positive_past_tense() {
        // "I fetched" / "I already called" should not trigger
        assert!(!eval_python_bool(
            r#"signals_tool_intent("I already completed the needed action call by fetching current news feeds.")"#
        ));
        assert!(!eval_python_bool(
            r#"signals_tool_intent("Current status from the live feeds I fetched:")"#
        ));
    }

    #[test]
    fn signals_tool_intent_no_false_positive_offer() {
        // "If you want, I can fetch..." uses "I can" which is not a V1 prefix
        assert!(!eval_python_bool(
            r#"signals_tool_intent("If you want, I can next fetch a cleaner update.")"#
        ));
    }

    #[tokio::test]
    async fn load_orchestrator_without_store_returns_default() {
        let (code, version) = load_orchestrator(None, ProjectId::new(), true).await;
        assert_eq!(version, 0);
        assert!(code.contains("run_loop"));
        assert!(code.contains("__llm_complete__"));
    }

    #[tokio::test]
    async fn load_orchestrator_with_runtime_version() {
        let project_id = ProjectId::new();
        let mut doc = MemoryDoc::new(
            project_id,
            "system",
            DocType::Note,
            ORCHESTRATOR_TITLE,
            "custom_orchestrator_code()",
        )
        .with_tags(vec![ORCHESTRATOR_TAG.to_string()]);
        doc.metadata = serde_json::json!({"version": 1});

        let store = Arc::new(crate::tests::InMemoryStore::with_docs(vec![doc]));
        let (code, version) =
            load_orchestrator(Some(&(store as Arc<dyn Store>)), project_id, true).await;
        assert_eq!(version, 1);
        assert!(code.contains("custom_orchestrator_code"));
    }

    #[tokio::test]
    async fn load_orchestrator_picks_highest_version() {
        let project_id = ProjectId::new();
        let mut doc_v1 = MemoryDoc::new(
            project_id,
            "system",
            DocType::Note,
            ORCHESTRATOR_TITLE,
            "v1_code()",
        )
        .with_tags(vec![ORCHESTRATOR_TAG.to_string()]);
        doc_v1.metadata = serde_json::json!({"version": 1});

        let mut doc_v3 = MemoryDoc::new(
            project_id,
            "system",
            DocType::Note,
            ORCHESTRATOR_TITLE,
            "v3_code()",
        )
        .with_tags(vec![ORCHESTRATOR_TAG.to_string()]);
        doc_v3.metadata = serde_json::json!({"version": 3});

        let mut doc_v2 = MemoryDoc::new(
            project_id,
            "system",
            DocType::Note,
            ORCHESTRATOR_TITLE,
            "v2_code()",
        )
        .with_tags(vec![ORCHESTRATOR_TAG.to_string()]);
        doc_v2.metadata = serde_json::json!({"version": 2});

        let store = Arc::new(crate::tests::InMemoryStore::with_docs(vec![
            doc_v1, doc_v3, doc_v2,
        ]));
        let (code, version) =
            load_orchestrator(Some(&(store as Arc<dyn Store>)), project_id, true).await;
        assert_eq!(version, 3);
        assert!(code.contains("v3_code"));
    }

    #[tokio::test]
    async fn rollback_after_max_failures() {
        let project_id = ProjectId::new();

        // Create v2 orchestrator
        let mut doc_v2 = MemoryDoc::new(
            project_id,
            "system",
            DocType::Note,
            ORCHESTRATOR_TITLE,
            "v2_buggy()",
        )
        .with_tags(vec![ORCHESTRATOR_TAG.to_string()]);
        doc_v2.metadata = serde_json::json!({"version": 2});

        // Create v1 orchestrator (fallback)
        let mut doc_v1 = MemoryDoc::new(
            project_id,
            "system",
            DocType::Note,
            ORCHESTRATOR_TITLE,
            "v1_stable()",
        )
        .with_tags(vec![ORCHESTRATOR_TAG.to_string()]);
        doc_v1.metadata = serde_json::json!({"version": 1});

        // Create failure tracker showing v2 has 3 failures
        let tracker = MemoryDoc::new(
            project_id,
            "system",
            DocType::Note,
            FAILURE_TRACKER_TITLE,
            r#"{"version": 2, "count": 3}"#,
        )
        .with_tags(vec!["orchestrator_meta".to_string()]);

        let store = Arc::new(crate::tests::InMemoryStore::with_docs(vec![
            doc_v2, doc_v1, tracker,
        ]));
        let (code, version) =
            load_orchestrator(Some(&(store as Arc<dyn Store>)), project_id, true).await;

        // Should skip v2 (too many failures) and load v1
        assert_eq!(version, 1);
        assert!(code.contains("v1_stable"));
    }

    #[tokio::test]
    async fn rollback_to_default_when_all_versions_fail() {
        let project_id = ProjectId::new();

        // Single version with 3 failures
        let mut doc_v1 = MemoryDoc::new(
            project_id,
            "system",
            DocType::Note,
            ORCHESTRATOR_TITLE,
            "v1_broken()",
        )
        .with_tags(vec![ORCHESTRATOR_TAG.to_string()]);
        doc_v1.metadata = serde_json::json!({"version": 1});

        let tracker = MemoryDoc::new(
            project_id,
            "system",
            DocType::Note,
            FAILURE_TRACKER_TITLE,
            r#"{"version": 1, "count": 5}"#,
        )
        .with_tags(vec!["orchestrator_meta".to_string()]);

        let store = Arc::new(crate::tests::InMemoryStore::with_docs(vec![
            doc_v1, tracker,
        ]));
        let (code, version) =
            load_orchestrator(Some(&(store as Arc<dyn Store>)), project_id, true).await;

        // Should fall back to compiled-in default (v0)
        assert_eq!(version, 0);
        assert!(code.contains("run_loop"));
    }

    #[tokio::test]
    async fn record_and_reset_failures() {
        let project_id = ProjectId::new();
        let store: Arc<dyn Store> = Arc::new(crate::tests::InMemoryStore::with_docs(vec![]));

        // Record 3 failures
        record_orchestrator_failure(&store, project_id, 2).await;
        record_orchestrator_failure(&store, project_id, 2).await;
        record_orchestrator_failure(&store, project_id, 2).await;

        let docs = store.list_shared_memory_docs(project_id).await.unwrap();
        let count = load_failure_count(&docs);
        assert_eq!(count, 3);

        // Reset
        reset_orchestrator_failures(&store, project_id).await;
        let docs = store.list_shared_memory_docs(project_id).await.unwrap();
        let count = load_failure_count(&docs);
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn failure_count_resets_on_new_version() {
        let project_id = ProjectId::new();
        let store: Arc<dyn Store> = Arc::new(crate::tests::InMemoryStore::with_docs(vec![]));

        // Record failures for version 1
        record_orchestrator_failure(&store, project_id, 1).await;
        record_orchestrator_failure(&store, project_id, 1).await;

        // Switch to version 2 — count should reset to 1
        record_orchestrator_failure(&store, project_id, 2).await;

        let docs = store.list_shared_memory_docs(project_id).await.unwrap();
        let count = load_failure_count(&docs);
        assert_eq!(count, 1);
    }

    #[test]
    fn normalize_pause_outcome_transitions_thread_to_waiting() {
        let mut thread = Thread::new(
            "goal",
            crate::types::thread::ThreadType::Foreground,
            ProjectId::new(),
            "user",
            crate::types::thread::ThreadConfig::default(),
        );
        thread.transition_to(ThreadState::Running, None).unwrap();

        let outcome = ThreadOutcome::GatePaused {
            gate_name: "approval".into(),
            action_name: "shell".into(),
            call_id: "call-1".into(),
            parameters: serde_json::json!({"cmd":"ls"}),
            resume_kind: crate::gate::ResumeKind::Approval { allow_always: true },
            resume_output: None,
        };
        normalize_pause_outcome(&mut thread, &outcome).unwrap();
        assert_eq!(thread.state, ThreadState::Waiting);
    }

    #[test]
    fn parse_outcome_completed() {
        let result = serde_json::json!({"outcome": "completed", "response": "Hello!"});
        let outcome = parse_outcome(&result);
        assert!(matches!(outcome, ThreadOutcome::Completed { response: Some(r) } if r == "Hello!"));
    }

    #[test]
    fn parse_outcome_failed() {
        let result = serde_json::json!({"outcome": "failed", "error": "boom"});
        let outcome = parse_outcome(&result);
        assert!(matches!(outcome, ThreadOutcome::Failed { error } if error == "boom"));
    }

    #[test]
    fn parse_outcome_gate_paused() {
        let result = serde_json::json!({
            "outcome": "gate_paused",
            "gate_name": "approval",
            "action_name": "shell",
            "call_id": "abc",
            "parameters": {"cmd": "rm -rf /"},
            "resume_kind": {"Approval": {"allow_always": true}}
        });
        let outcome = parse_outcome(&result);
        assert!(
            matches!(outcome, ThreadOutcome::GatePaused { action_name, .. } if action_name == "shell")
        );
    }

    #[test]
    fn parse_outcome_max_iterations() {
        let result = serde_json::json!({"outcome": "max_iterations"});
        let outcome = parse_outcome(&result);
        assert!(matches!(outcome, ThreadOutcome::MaxIterations));
    }

    #[test]
    fn parse_outcome_stopped() {
        let result = serde_json::json!({"outcome": "stopped"});
        let outcome = parse_outcome(&result);
        assert!(matches!(outcome, ThreadOutcome::Stopped));
    }
}
