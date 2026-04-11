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
use std::sync::atomic::{AtomicU64, Ordering};

use std::collections::HashMap;

use monty::{
    ExtFunctionResult, LimitedTracker, MontyObject, MontyRun, NameLookupResult, PrintWriter,
    ResourceLimits, RunProgress,
};
use tracing::{debug, warn};

use crate::capability::lease::LeaseManager;
use crate::capability::policy::PolicyEngine;
use crate::memory::RetrievalEngine;
use crate::runtime::lease_refresh::reconcile_dynamic_tool_lease;
use crate::runtime::messaging::{SignalReceiver, ThreadOutcome, ThreadSignal};
use crate::traits::effect::{EffectExecutor, ThreadExecutionContext};
use crate::traits::llm::{LlmBackend, LlmCallConfig};
use crate::traits::store::Store;
use crate::types::error::EngineError;
use crate::types::event::{EventKind, ThreadEvent, summarize_params};
use crate::types::message::ThreadMessage;
use crate::types::project::ProjectId;
use crate::types::shared_owner_id;
use crate::types::step::{ActionCall, StepId, TokenUsage};
use crate::types::thread::{ActiveSkillProvenance, Thread, ThreadState};
use ironclaw_common::ValidTimezone;

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

/// Extract and validate user_timezone from thread metadata (set by bridge router).
fn thread_user_timezone(thread: &Thread) -> Option<ValidTimezone> {
    thread
        .metadata
        .get("user_timezone")
        .and_then(|v| v.as_str())
        .and_then(ValidTimezone::parse)
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
const LEASE_REFRESH_WARN_INTERVAL_SECS: u64 = 60;

fn warn_on_lease_refresh_failure(context: &'static str, error: &crate::types::error::EngineError) {
    static LAST_WARN_TS: AtomicU64 = AtomicU64::new(0);

    let now = chrono::Utc::now().timestamp().max(0) as u64;
    let last = LAST_WARN_TS.load(Ordering::Relaxed);
    if now.saturating_sub(last) >= LEASE_REFRESH_WARN_INTERVAL_SECS
        && LAST_WARN_TS
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        warn!(context, error = %error, "dynamic lease refresh failed");
    } else {
        debug!(context, error = %error, "dynamic lease refresh failed");
    }
}

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
                            LlmCompleteDeps {
                                llm,
                                effects,
                                leases,
                                store,
                            },
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
                    "__get_actions__" => handle_get_actions(thread, effects, leases, store).await,

                    // __list_skills__(max_candidates, max_tokens)
                    "__list_skills__" => handle_list_skills(args, thread, store).await,

                    // __record_skill_usage__(doc_id, success)
                    "__record_skill_usage__" => handle_record_skill_usage(args, store).await,

                    // __regex_match__(pattern, text) -> bool
                    // Evaluates a regex against text using Rust's regex crate.
                    // Invalid patterns return False silently. Monty has no `re`
                    // module, so this host function bridges the gap for the
                    // skill selector's pattern-based scoring.
                    "__regex_match__" => handle_regex_match(args),

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

struct LlmCompleteDeps<'a> {
    llm: &'a Arc<dyn LlmBackend>,
    effects: &'a Arc<dyn EffectExecutor>,
    leases: &'a Arc<LeaseManager>,
    store: Option<&'a Arc<dyn Store>>,
}

/// Handle `__llm_complete__(messages, actions, config)`.
///
/// Calls the LLM and returns the response as a dict:
/// `{type: "text"|"code"|"actions", content/code/calls: ..., usage: {...}}`
///
async fn handle_llm_complete(
    args: &[MontyObject],
    _kwargs: &[(MontyObject, MontyObject)],
    thread: &mut Thread,
    deps: LlmCompleteDeps<'_>,
    total_tokens: &mut TokenUsage,
) -> ExtFunctionResult {
    use crate::types::step::LlmResponse;

    let explicit_messages = args.first().map(monty_to_json).filter(|v| !v.is_null());
    let explicit_config = args.get(2).map(monty_to_json).filter(|v| !v.is_null());
    let messages = explicit_messages
        .as_ref()
        .and_then(json_to_thread_messages)
        .unwrap_or_else(|| thread.messages.clone());

    if let Err(e) = reconcile_dynamic_tool_lease(
        thread,
        deps.effects,
        deps.leases,
        deps.store,
        &crate::LeasePlanner::new(),
    )
    .await
    {
        warn_on_lease_refresh_failure("llm_complete", &e);
    }

    let active_leases = deps.leases.active_for_thread(thread.id).await;
    let actions = deps
        .effects
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

    match deps.llm.complete(&messages, &actions, &config).await {
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
                    // Single source of truth for the Python interchange
                    // shape — must round-trip via `python_json_to_action_calls`.
                    let calls_json = action_calls_to_python_json(&calls);
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
        user_timezone: thread_user_timezone(thread),
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
            // If the CodeAct snippet itself failed (Python SyntaxError, runtime
            // error, etc.), surface it as an ActionFailed event so traces and
            // observers see the failure. Without this, parse errors silently
            // fall back to the LLM via the result dict and never warn callers.
            if result.had_error {
                let error_msg = if !result.stdout.is_empty() {
                    let snippet: String = result.stdout.chars().take(500).collect();
                    format!("CodeAct execution failed: {snippet}")
                } else {
                    "CodeAct execution failed (no stdout)".to_string()
                };
                let failed_event = ThreadEvent::new(
                    thread.id,
                    EventKind::ActionFailed {
                        step_id: exec_ctx.step_id,
                        action_name: "__codeact__".to_string(),
                        // Synthetic call_id derived from the step id —
                        // CodeAct snippet failures don't have an LLM-provided
                        // call_id, but `loop_engine.rs:1277` asserts that
                        // ActionFailed events carry a non-empty call_id for
                        // trace correlation.
                        call_id: format!("codeact-step-{}", exec_ctx.step_id.0),
                        error: error_msg,
                        params_summary: None,
                    },
                );
                if let Some(tx) = event_tx {
                    let _ = tx.send(failed_event.clone());
                }
                thread.events.push(failed_event);
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
        user_timezone: thread_user_timezone(thread),
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

    // 3. Atomically re-find + consume a lease use under a single write
    // lock. This closes the TOCTOU window between the read-only
    // `find_lease_for_action` (used above for the policy check) and the
    // consume — without it, two concurrent calls could both observe a
    // lease with one remaining use and both proceed to execute. Mirrors
    // `structured.rs::execute_action_batch_with_results`.
    let lease = match leases.find_and_consume(thread.id, &name).await {
        Ok(l) => l,
        Err(e) => {
            debug!(error = %e, "atomic lease find_and_consume failed");
            let error = format!("lease consumption failed for action '{name}': {e}");
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

    // 4. Execute
    let ps = summarize_params(&name, &params);
    match effects
        .execute_action(&name, params, &lease, &exec_ctx)
        .await
    {
        Ok(r) => {
            // Effect adapters wrap tool errors as `Ok(ActionResult { is_error: true })`
            // — surface them as `ActionFailed` so traces and observers see the
            // failure. See `resolve_tool_future` in `scripting.rs` for the same
            // pattern on the structured-tool path.
            if r.is_error {
                let error_msg = r
                    .output
                    .get("error")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| r.output.to_string());
                emit_and_record(
                    thread,
                    event_tx,
                    EventKind::ActionFailed {
                        step_id: exec_ctx.step_id,
                        action_name: name.clone(),
                        call_id: call_id.clone(),
                        error: error_msg,
                        params_summary: ps.clone(),
                    },
                    &call_id,
                    &name,
                    &r.output,
                );
            } else {
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
            }
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

        // Atomically re-find + consume a lease use under a single write
        // lock, closing the TOCTOU window between the read-only
        // `find_lease_for_action` above and the consume. Mirrors
        // `structured.rs::execute_action_batch_with_results`.
        let lease = match leases.find_and_consume(thread.id, &pc.name).await {
            Ok(l) => l,
            Err(e) => {
                debug!(error = %e, "atomic lease find_and_consume failed");
                let error = format!("lease consumption failed for action '{}': {e}", pc.name);
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
            // Read source_channel from thread metadata so downstream tools
            // (e.g. mission_create) can default notify_channels to the
            // originating channel. Hardcoding `None` here was a bug — it
            // silently dropped the gateway routing for any tool dispatched
            // through the parallel batch path.
            source_channel: thread_source_channel(thread),
            user_timezone: thread_user_timezone(thread),
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
        // Capture once outside the loop — the thread's metadata is stable
        // for the duration of the parallel batch.
        let parallel_source_channel = thread_source_channel(thread);
        let parallel_user_timezone = thread_user_timezone(thread);

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
                // See comment above — read from thread metadata, not None.
                source_channel: parallel_source_channel.clone(),
                user_timezone: parallel_user_timezone,
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
            // Surface wrapped errors as ActionFailed (see resolve_tool_future
            // and the parallel execute path for the same pattern).
            let event = if r.is_error {
                let error_msg = r
                    .output
                    .get("error")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| r.output.to_string());
                EventKind::ActionFailed {
                    step_id: exec_ctx.step_id,
                    action_name: name.to_string(),
                    call_id: call_id.to_string(),
                    error: error_msg,
                    params_summary: params_summary.clone(),
                }
            } else {
                EventKind::ActionExecuted {
                    step_id: exec_ctx.step_id,
                    action_name: name.to_string(),
                    call_id: call_id.to_string(),
                    duration_ms: r.duration.as_millis() as u64,
                    params_summary: params_summary.clone(),
                }
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
    thread: &mut Thread,
    effects: &Arc<dyn EffectExecutor>,
    leases: &Arc<LeaseManager>,
    store: Option<&Arc<dyn Store>>,
) -> ExtFunctionResult {
    if let Err(e) =
        reconcile_dynamic_tool_lease(thread, effects, leases, store, &crate::LeasePlanner::new())
            .await
    {
        warn_on_lease_refresh_failure("get_actions", &e);
    }

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

/// Handle `__regex_match__(pattern, text) -> bool`.
///
/// Compiles `pattern` with a bounded size limit and returns whether it
/// matches anywhere in `text`. Invalid regex or a size-limit violation
/// returns `False` silently. Used by the Python skill selector for regex
/// pattern scoring (Monty has no `re` module).
///
/// **Security: ReDoS safety.** This handler accepts arbitrary patterns from
/// the Python orchestrator (which itself receives them from skill manifests)
/// and runs them on user-supplied text. Safety relies on the `regex` crate's
/// linear-time matching guarantee (no backreferences, no lookaround) plus the
/// 64 KiB compiled-size cap and DFA-size cap below. If the `regex` crate is
/// ever swapped for `fancy-regex` (which supports backreferences and is NOT
/// linear-time), this becomes a real ReDoS vector. This is enforced by
/// convention and documentation only — see the top-of-crate comment in
/// `crates/ironclaw_engine/src/lib.rs`. (A `#[cfg(feature = "fancy-regex")]
/// compile_error!` tripwire was evaluated but conflicts with
/// `cargo clippy --all-features` which is the standard CI command.)
fn handle_regex_match(args: &[MontyObject]) -> ExtFunctionResult {
    let pattern = args.first().map(monty_to_string).unwrap_or_default();
    let text = args.get(1).map(monty_to_string).unwrap_or_default();
    if pattern.is_empty() {
        return ExtFunctionResult::Return(MontyObject::Bool(false));
    }
    // Cap compiled regex size to prevent ReDoS (matches the 64 KiB limit used
    // by `LoadedSkill::compile_patterns` in `ironclaw_skills`). Also cap the
    // lazy-DFA cache: the `regex` crate's DFA can grow beyond `size_limit`
    // during matching, so `dfa_size_limit` is a separate defensive cap on
    // memory allocation from a crafted pattern over untrusted skill manifests.
    const MAX_REGEX_SIZE: usize = 1 << 16;
    let matched = match regex::RegexBuilder::new(&pattern)
        .size_limit(MAX_REGEX_SIZE)
        .dfa_size_limit(MAX_REGEX_SIZE)
        .build()
    {
        Ok(re) => re.is_match(&text),
        Err(e) => {
            debug!("__regex_match__: invalid pattern '{pattern}': {e}");
            false
        }
    };
    ExtFunctionResult::Return(MontyObject::Bool(matched))
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
            // Serialize action_calls through the Python interchange shape
            // (`{name, call_id, params}`) so the bootstrap context is
            // round-trip compatible with `python_json_to_action_calls`.
            // Using bare `m.action_calls` here produces the canonical Rust
            // serde format (`{action_name, id, parameters}`), which the
            // Python orchestrator passes back verbatim on the next
            // `__llm_complete__` call — and `python_json_to_action_calls`
            // then fails with "missing field `name`", orphaning every
            // subsequent tool result. This is the SECOND code path (after
            // `handle_llm_complete`) that feeds action_calls into the
            // Python working transcript; both must use the same shape.
            let calls_json = m
                .action_calls
                .as_ref()
                .map(|calls| serde_json::Value::Array(action_calls_to_python_json(calls)));
            serde_json::json!({
                "role": format!("{:?}", m.role),
                "content": m.content,
                "action_name": m.action_name,
                "action_call_id": m.action_call_id,
                "action_calls": calls_json,
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

/// JSON shape used to interchange `ActionCall`s with the Python orchestrator.
///
/// This is the *single* place that defines the field naming convention used
/// across the Python boundary. It is intentionally separate from the
/// canonical `ActionCall` type because:
///
/// - `ActionCall` uses Rust-idiomatic field names (`id`, `action_name`,
///   `parameters`) and is also persisted into Step records and ThreadEvents.
///   Renaming its serde fields would invalidate every existing row.
/// - The Python orchestrator uses friendlier names (`call_id`, `name`,
///   `params`) that read naturally in CodeAct prompts and `default.py`.
///
/// Without this type, the round-trip is asymmetric: Rust → Python uses one
/// shape, Python → Rust used `serde_json::from_value::<Vec<ActionCall>>`
/// which silently fails (`.ok()` swallows the error) and produces `None`,
/// which means assistant messages came back without `action_calls`. The
/// downstream effect is that every tool result looks orphaned to
/// `sanitize_tool_messages` and gets rewritten as a user message — losing
/// the assistant ↔ tool_result linkage the LLM needs to reason about prior
/// tool calls.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PythonActionCall {
    name: String,
    call_id: String,
    params: serde_json::Value,
}

impl From<&ActionCall> for PythonActionCall {
    fn from(c: &ActionCall) -> Self {
        Self {
            name: c.action_name.clone(),
            call_id: c.id.clone(),
            params: c.parameters.clone(),
        }
    }
}

impl From<PythonActionCall> for ActionCall {
    fn from(p: PythonActionCall) -> Self {
        Self {
            id: p.call_id,
            action_name: p.name,
            parameters: p.params,
        }
    }
}

/// Serialize a slice of `ActionCall`s into the Python interchange shape.
///
/// On serialization failure (essentially unreachable for `String + String +
/// Value`, but still possible if the `serde_json::Value` parameters tree
/// contains a key whose stringification fails), the entry is **dropped**
/// from the output rather than replaced with `Value::Null`. The previous
/// `unwrap_or_else(|_| Value::Null)` corrupted the array — Python's
/// `default.py` accesses `c.get("name")` / `c.get("call_id")` /
/// `c.get("params")` on each entry, so a `null` would crash with a Python
/// `AttributeError` and lose the entire LLM step. `filter_map` produces a
/// shorter array, which Python's tool-result loop handles correctly because
/// it iterates `range(len(results))` against the shortened call list. The
/// warn log is preserved so operators have a breadcrumb if it ever fires.
fn action_calls_to_python_json(calls: &[ActionCall]) -> Vec<serde_json::Value> {
    calls
        .iter()
        .filter_map(|c| match serde_json::to_value(PythonActionCall::from(c)) {
            Ok(value) => Some(value),
            Err(e) => {
                warn!(
                    error = %e,
                    action_name = %c.action_name,
                    "Failed to serialize ActionCall for Python orchestrator — dropping entry"
                );
                None
            }
        })
        .collect()
}

/// Build a PII-safe summary of an `action_calls` JSON value for log output.
///
/// The action_calls payload contains tool parameters, which can carry user
/// PII (search queries, file names, email content, conversation text).
/// Dumping the full value into a `warn!` log would leak that PII to log
/// aggregation systems (Datadog, CloudWatch, Sentry) the moment the parser
/// fails — and the parser only fails when the Python ↔ Rust shape drifts,
/// which is exactly when an operator is most likely to be grepping logs.
///
/// We emit only the structural information operators actually need to
/// debug a shape drift: array length and the keys of the first entry. The
/// keys themselves are not user data — they're field names like
/// `name`/`call_id`/`params` that are static across all calls.
fn summarize_action_calls_for_log(value: &serde_json::Value) -> String {
    match value.as_array() {
        Some(arr) if arr.is_empty() => "empty array".to_string(),
        Some(arr) => {
            let first_keys = arr
                .first()
                .and_then(|v| v.as_object())
                .map(|obj| {
                    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
                    keys.sort_unstable();
                    keys.join(",")
                })
                .unwrap_or_else(|| "<not an object>".to_string());
            format!(
                "array of {} entries; first entry keys: [{}]",
                arr.len(),
                first_keys
            )
        }
        None => format!("non-array value of type {}", json_value_type_name(value)),
    }
}

/// Cheap type-name string for a `serde_json::Value`. Used by
/// `summarize_action_calls_for_log` to surface the wrong-shape case
/// (e.g. Python passed a string instead of an array) without leaking the
/// actual contents.
fn json_value_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Deserialize an `action_calls` JSON array (in Python interchange shape)
/// back into canonical `ActionCall`s.
///
/// Logs a warning on failure rather than swallowing silently. The whole
/// commit that introduced this helper exists to undo a `.ok()` swallow that
/// dropped action_calls without any signal — replacing it with another
/// `.ok()?` would re-introduce the same trap, just one layer deeper. If the
/// shape ever drifts again (Python orchestrator field rename, extra
/// required field, partial migration), the warning is the operator-visible
/// breadcrumb that explains why subsequent tool results suddenly look
/// orphaned to `sanitize_tool_messages`.
///
/// The warn log emits a structural summary (`summarize_action_calls_for_log`)
/// instead of the raw value because tool parameters can contain user PII.
fn python_json_to_action_calls(value: &serde_json::Value) -> Option<Vec<ActionCall>> {
    match serde_json::from_value::<Vec<PythonActionCall>>(value.clone()) {
        Ok(parsed) => Some(parsed.into_iter().map(ActionCall::from).collect()),
        Err(e) => {
            warn!(
                error = %e,
                shape = %summarize_action_calls_for_log(value),
                "Failed to parse action_calls from Python orchestrator — \
                 assistant message will lose tool_call linkage and downstream \
                 tool results will be rewritten as user messages"
            );
            None
        }
    }
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
        // Filter out null before calling the parser — `action_calls: null`
        // is Python's legitimate "this message has no tool calls" signal (text
        // response), not a parse error. Without this filter, the warn log in
        // python_json_to_action_calls fires on every text-only assistant
        // message with "invalid type: null, expected a sequence".
        let action_calls = item
            .get("action_calls")
            .filter(|v| !v.is_null())
            .and_then(python_json_to_action_calls);

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
        let helpers = &DEFAULT_ORCHESTRATOR[..helpers_end]; // safety: find() returns a char boundary on this ASCII-only constant

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
                    // Dispatch the real host functions the test exercises so
                    // e.g. `__regex_match__` routes through the production
                    // handler instead of being stubbed out to `None`.
                    let ext_result = match call.function_name.as_str() {
                        "__regex_match__" => handle_regex_match(&call.args),
                        _ => ExtFunctionResult::Return(MontyObject::None),
                    };
                    progress = call
                        .resume(ext_result, PrintWriter::Collect(&mut stdout))
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

    // ── __regex_match__ host function reachability ───────────────

    #[test]
    fn regex_match_host_function_is_callable_from_monty() {
        // Regression test for PR #1736 review (serrrfirat, 3059161877):
        // verify that Monty's NameLookup + FunctionCall dispatch actually
        // reaches `handle_regex_match` when default.py calls
        // `__regex_match__(...)`. If Monty ever starts resolving the name
        // before the call, this test will fail with a NameError.
        assert!(eval_python_bool(
            r#"bool(__regex_match__("abc", "xxabcxx"))"#
        ));
        assert!(!eval_python_bool(
            r#"bool(__regex_match__("zzz", "xxabcxx"))"#
        ));
        // Invalid pattern should return false silently (the host function
        // swallows the compile error).
        assert!(!eval_python_bool(r#"bool(__regex_match__("[", "abc"))"#));
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

    // ── Python ↔ Rust ActionCall round-trip ───────────────────────────────
    //
    // Regression tests for the orphaned-tool-result bug. The Python
    // orchestrator stores `action_calls` on assistant messages using the
    // shape `{name, call_id, params}`, but the canonical Rust `ActionCall`
    // uses `{action_name, id, parameters}`. Without the explicit
    // `PythonActionCall` interchange type, `serde_json::from_value` would
    // silently fail (`.ok()` swallows the error) and the Python-shaped
    // assistant message would be parsed back as a plain assistant message
    // with no tool calls, causing every subsequent ActionResult to be
    // detected as orphaned by `sanitize_tool_messages` in the host crate.

    #[test]
    fn python_action_call_round_trips_through_serde() {
        let original = ActionCall {
            id: "call_abc123".to_string(),
            action_name: "google_drive_tool".to_string(),
            parameters: serde_json::json!({"query": "expenses"}),
        };

        let python_json = serde_json::to_value(PythonActionCall::from(&original))
            .expect("PythonActionCall must serialize");
        // Python-friendly field names — match what default.py reads.
        assert_eq!(python_json["name"], "google_drive_tool");
        assert_eq!(python_json["call_id"], "call_abc123");
        assert_eq!(
            python_json["params"],
            serde_json::json!({"query": "expenses"})
        );

        let parsed: PythonActionCall =
            serde_json::from_value(python_json).expect("must deserialize");
        let round_tripped: ActionCall = parsed.into();
        assert_eq!(round_tripped.id, original.id);
        assert_eq!(round_tripped.action_name, original.action_name);
        assert_eq!(round_tripped.parameters, original.parameters);
    }

    #[test]
    fn action_calls_to_python_json_uses_python_field_names() {
        let calls = vec![
            ActionCall {
                id: "call_1".to_string(),
                action_name: "notion_notion_search".to_string(),
                parameters: serde_json::json!({"query": "name"}),
            },
            ActionCall {
                id: "call_2".to_string(),
                action_name: "google_drive_tool".to_string(),
                parameters: serde_json::json!({"action": "list"}),
            },
        ];
        let json = action_calls_to_python_json(&calls);
        assert_eq!(json.len(), 2);
        assert_eq!(json[0]["name"], "notion_notion_search");
        assert_eq!(json[0]["call_id"], "call_1");
        assert_eq!(json[1]["name"], "google_drive_tool");
        assert_eq!(json[1]["call_id"], "call_2");
    }

    #[test]
    fn python_json_to_action_calls_parses_python_field_names() {
        // The exact shape default.py produces (and stores on assistant
        // messages via `append_message(..., action_calls=calls)`).
        let python_json = serde_json::json!([
            {"name": "notion_notion_search", "call_id": "call_xyz", "params": {"q": "foo"}},
            {"name": "google_drive_tool", "call_id": "call_abc", "params": {"action": "list"}},
        ]);
        let parsed = python_json_to_action_calls(&python_json).expect("must parse");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].action_name, "notion_notion_search");
        assert_eq!(parsed[0].id, "call_xyz");
        assert_eq!(parsed[0].parameters, serde_json::json!({"q": "foo"}));
        assert_eq!(parsed[1].action_name, "google_drive_tool");
        assert_eq!(parsed[1].id, "call_abc");
    }

    #[test]
    fn python_json_to_action_calls_rejects_canonical_field_names() {
        // Sanity check: the parser is strict about Python field names.
        // If `default.py` ever changes the shape, the test must catch it.
        let canonical_json = serde_json::json!([
            {"action_name": "search", "id": "call_x", "parameters": {}}
        ]);
        // Missing "name", "call_id", "params" → returns None.
        assert!(python_json_to_action_calls(&canonical_json).is_none());
    }

    #[test]
    fn summarize_action_calls_for_log_does_not_leak_user_pii() {
        // The whole point of this helper is that the warn log path on a
        // shape-drift failure must NOT dump tool parameters (which can
        // contain user PII like search queries, file names, email content)
        // into log aggregation systems. The summary should expose only
        // structural information: array length and the keys of the first
        // entry. The keys themselves are static (`name`, `call_id`,
        // `params`), not user data.
        let pii_value = serde_json::json!([
            {
                "name": "google_drive_tool",
                "call_id": "call_xyz",
                "params": {
                    "query": "salary spreadsheet for joe",
                    "secret_token": "very-sensitive-token-do-not-log"
                }
            },
            {
                "name": "gmail",
                "call_id": "call_abc",
                "params": {
                    "subject": "private message about layoffs"
                }
            }
        ]);
        let summary = summarize_action_calls_for_log(&pii_value);

        // Structural info present.
        assert!(summary.contains("array of 2 entries"));
        assert!(summary.contains("call_id"));
        assert!(summary.contains("name"));
        assert!(summary.contains("params"));

        // PII fields and their values must NOT appear.
        assert!(
            !summary.contains("salary"),
            "summary must not leak user PII from params: {summary}"
        );
        assert!(
            !summary.contains("very-sensitive-token"),
            "summary must not leak credential-shaped values: {summary}"
        );
        assert!(
            !summary.contains("layoffs"),
            "summary must not leak free-text content: {summary}"
        );
        assert!(
            !summary.contains("google_drive_tool"),
            "summary must not leak the tool name itself (could expose intent): {summary}"
        );
    }

    #[test]
    fn summarize_action_calls_for_log_handles_edge_cases() {
        assert_eq!(
            summarize_action_calls_for_log(&serde_json::json!([])),
            "empty array"
        );
        assert!(
            summarize_action_calls_for_log(&serde_json::json!("not an array")).contains("string")
        );
        assert!(
            summarize_action_calls_for_log(&serde_json::json!({"foo": "bar"})).contains("object")
        );
        assert!(summarize_action_calls_for_log(&serde_json::json!(null)).contains("null"));
    }

    /// Caller-level regression test: feeds `json_to_thread_messages` the
    /// exact JSON shape that `default.py` produces for an assistant message
    /// with tool calls followed by tool results, and asserts that the
    /// resulting `ThreadMessage`s preserve the `action_calls` ↔
    /// `action_call_id` linkage. Without the `PythonActionCall` parser the
    /// assistant message would come back with `action_calls = None` and
    /// every following ActionResult would look orphaned to the bridge.
    #[test]
    fn json_to_thread_messages_preserves_action_calls_from_python_orchestrator() {
        // This is the literal shape `default.py` writes into
        // `state["working_messages"]` after a Tier 0 step:
        //
        //   append_message(working_messages, "Assistant", "...", action_calls=calls)
        //   append_message(working_messages, "ActionResult", "...", action_name=..., action_call_id=...)
        //
        // where `calls` came from the LLM response and has shape
        // `[{"name": ..., "call_id": ..., "params": ...}]`.
        let working_messages = serde_json::json!([
            {"role": "User", "content": "search in notion for my name"},
            {
                "role": "Assistant",
                "content": "",
                "action_calls": [
                    {
                        "name": "notion_notion_search",
                        "call_id": "call_xyz",
                        "params": {"query": "Illia"}
                    }
                ]
            },
            {
                "role": "ActionResult",
                "content": "found 3 results",
                "action_name": "notion_notion_search",
                "action_call_id": "call_xyz"
            }
        ]);

        let messages = json_to_thread_messages(&working_messages).expect("must parse");
        assert_eq!(messages.len(), 3);

        // The assistant message MUST have action_calls populated, with
        // matching call_id. If this assertion fails, the bridge layer
        // will treat the following ActionResult as orphaned and rewrite
        // it as a user message — losing the model's ability to reason
        // about prior tool output.
        let assistant = &messages[1];
        assert_eq!(
            assistant.role,
            crate::types::message::MessageRole::Assistant
        );
        let calls = assistant
            .action_calls
            .as_ref()
            .expect("assistant message must carry action_calls after round-trip");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_xyz");
        assert_eq!(calls[0].action_name, "notion_notion_search");
        assert_eq!(calls[0].parameters, serde_json::json!({"query": "Illia"}));

        // The ActionResult must reference the same call_id so the bridge
        // can pair them.
        let result = &messages[2];
        assert_eq!(
            result.role,
            crate::types::message::MessageRole::ActionResult
        );
        assert_eq!(result.action_call_id.as_deref(), Some("call_xyz"));
        assert_eq!(result.action_name.as_deref(), Some("notion_notion_search"));
    }

    /// Regression for the gate-resume / bootstrap path: when a thread
    /// resumes after approval or auth, `build_orchestrator_inputs`
    /// serializes `thread.internal_messages` into the bootstrap context
    /// that Python reads into `working_messages`. If `action_calls` is
    /// serialized with canonical `ActionCall` field names (`action_name`,
    /// `id`, `parameters`) instead of the Python interchange names
    /// (`name`, `call_id`, `params`), the next `__llm_complete__` call
    /// passes them back through `json_to_thread_messages` which fails
    /// with "missing field `name`" and orphans every subsequent tool
    /// result.
    ///
    /// This test simulates the full round-trip: build a `ThreadMessage`
    /// with action_calls → serialize through `build_orchestrator_inputs`'s
    /// exact serialization pattern → parse back through
    /// `json_to_thread_messages` → assert the calls survive. If anyone
    /// adds a THIRD serialization path in the future and uses canonical
    /// names, this test documents the pattern they should follow.
    #[test]
    fn bootstrap_context_action_calls_round_trip_through_python_interchange() {
        // Build a thread message the way the engine does: an assistant
        // message with action_calls in canonical ActionCall format (the
        // shape stored in the DB / internal_messages).
        let msg = ThreadMessage::assistant_with_actions(
            Some("I'll search for that".to_string()),
            vec![ActionCall {
                id: "call_resume_test".to_string(),
                action_name: "google_drive_tool".to_string(),
                parameters: serde_json::json!({"query": "budget"}),
            }],
        );

        // Serialize through the SAME pattern `build_orchestrator_inputs`
        // uses. This is the exact code path that was broken before the
        // fix — it was using `"action_calls": m.action_calls` which
        // produced canonical field names.
        let calls_json = msg
            .action_calls
            .as_ref()
            .map(|calls| serde_json::Value::Array(action_calls_to_python_json(calls)));
        let serialized = serde_json::json!([{
            "role": "Assistant",
            "content": msg.content,
            "action_name": msg.action_name,
            "action_call_id": msg.action_call_id,
            "action_calls": calls_json,
        }]);

        // Parse back through the same path Python's working_messages
        // takes when it calls __llm_complete__.
        let parsed = json_to_thread_messages(&serialized).expect("must parse");
        assert_eq!(parsed.len(), 1);

        let assistant = &parsed[0];
        let calls = assistant.action_calls.as_ref().expect(
            "bootstrap context action_calls must survive the round-trip. \
                 If this fails, a serialization path is using canonical ActionCall \
                 field names instead of PythonActionCall interchange names.",
        );
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_resume_test");
        assert_eq!(calls[0].action_name, "google_drive_tool");
        assert_eq!(calls[0].parameters, serde_json::json!({"query": "budget"}));
    }

    /// Negative regression: verify that canonical ActionCall field names
    /// do NOT round-trip. If this test ever PASSES, it means someone
    /// added `#[serde(rename)]` to ActionCall or changed the parser to
    /// accept both formats — which is fine, but the PythonActionCall
    /// interchange type can then be removed. This test documents the
    /// current contract: canonical names are rejected by the parser.
    #[test]
    fn canonical_action_call_field_names_do_not_round_trip() {
        let serialized_with_canonical_names = serde_json::json!([{
            "role": "Assistant",
            "content": "",
            "action_calls": [{
                "action_name": "search",
                "id": "call_x",
                "parameters": {}
            }],
        }]);
        let parsed =
            json_to_thread_messages(&serialized_with_canonical_names).expect("messages parse");
        // The assistant message should have NO action_calls because the
        // parser rejects canonical field names.
        assert!(
            parsed[0].action_calls.is_none(),
            "canonical ActionCall field names must NOT parse as action_calls. \
             If this assertion fails, the PythonActionCall interchange type \
             is no longer needed — either remove it or update the contract."
        );
    }

    /// Regression: `action_calls: null` is Python's legitimate "this
    /// message has no tool calls" signal (text-only response). Before the
    /// null filter, `python_json_to_action_calls` would fire a warn log
    /// with "invalid type: null, expected a sequence" on every text-only
    /// assistant message — a false alarm that masked real drift issues.
    #[test]
    fn json_to_thread_messages_handles_null_action_calls_gracefully() {
        let messages = serde_json::json!([
            {
                "role": "Assistant",
                "content": "Here is your answer.",
                "action_calls": null
            }
        ]);
        let parsed = json_to_thread_messages(&messages).expect("must parse");
        assert_eq!(parsed.len(), 1);
        assert_eq!(
            parsed[0].role,
            crate::types::message::MessageRole::Assistant
        );
        assert_eq!(parsed[0].content, "Here is your answer.");
        assert!(
            parsed[0].action_calls.is_none(),
            "null action_calls must produce None, not a parse error"
        );
    }

    /// Verify that messages WITHOUT the action_calls key at all (the most
    /// common case for text responses) also parse correctly — this is the
    /// baseline that the null-filtering regression test extends.
    #[test]
    fn json_to_thread_messages_handles_absent_action_calls() {
        let messages = serde_json::json!([
            {"role": "Assistant", "content": "Just text, no tools."}
        ]);
        let parsed = json_to_thread_messages(&messages).expect("must parse");
        assert_eq!(parsed.len(), 1);
        assert!(parsed[0].action_calls.is_none());
    }

    /// Empty action_calls array is valid (LLM decided not to call any
    /// tools this turn but the response still has the array field). Must
    /// produce `Some(vec![])`, not `None`.
    #[test]
    fn json_to_thread_messages_handles_empty_action_calls_array() {
        let messages = serde_json::json!([
            {
                "role": "Assistant",
                "content": "No tools needed.",
                "action_calls": []
            }
        ]);
        let parsed = json_to_thread_messages(&messages).expect("must parse");
        assert_eq!(parsed.len(), 1);
        let calls = parsed[0]
            .action_calls
            .as_ref()
            .expect("empty array should produce Some(vec![])");
        assert!(calls.is_empty());
    }
}
