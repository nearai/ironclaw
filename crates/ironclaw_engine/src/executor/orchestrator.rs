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
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use std::collections::HashMap;

use monty::{
    ExtFunctionResult, LimitedTracker, MontyObject, MontyRun, NameLookupResult, PrintWriter,
    ResourceLimits, RunProgress,
};
use tracing::{debug, warn};

use super::scripting::{execute_code, json_to_monty, monty_to_json, monty_to_string};
use super::thread_context::thread_execution_context;
use crate::capability::lease::LeaseManager;
use crate::capability::policy::PolicyEngine;
use crate::memory::RetrievalEngine;
use crate::runtime::lease_refresh::reconcile_dynamic_tool_lease;
use crate::runtime::messaging::{SignalReceiver, ThreadOutcome, ThreadSignal};
use crate::traits::effect::{EffectExecutor, ThreadExecutionContext};
use crate::traits::llm::{LlmBackend, LlmCallConfig};
use crate::traits::store::Store;
use crate::types::error::{EngineError, OrchestratorFailure, OrchestratorFailureKind};
use crate::types::event::{EventKind, ThreadEvent, summarize_params};
use crate::types::message::ThreadMessage;
use crate::types::project::ProjectId;
use crate::types::shared_owner_id;
use crate::types::step::{ActionCall, StepId, TokenUsage};
use crate::types::thread::{ActiveSkillProvenance, Thread, ThreadState};

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

fn apply_snapshot_inventory(
    exec_ctx: &mut ThreadExecutionContext,
    inventory: Option<Arc<crate::types::capability::ActionInventory>>,
) -> Arc<[crate::types::capability::ActionDef]> {
    let available_actions: Arc<[crate::types::capability::ActionDef]> = inventory
        .as_ref()
        .map(|inventory| inventory.inline.clone().into())
        .unwrap_or_else(|| Arc::from([]));
    if let Some(inventory) = inventory {
        exec_ctx.available_actions_snapshot = Some(Arc::clone(&available_actions));
        exec_ctx.available_action_inventory_snapshot = Some(inventory);
    }
    available_actions
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

/// Default orchestrator VM wall-clock budget, in seconds.
const ORCHESTRATOR_DEFAULT_MAX_DURATION_SECS: u64 = 300;
/// Floor for the configurable orchestrator budget, to prevent nonsense values.
const ORCHESTRATOR_MIN_MAX_DURATION_SECS: u64 = 30;
/// Ceiling for the configurable orchestrator budget, bounding resource waste.
const ORCHESTRATOR_MAX_MAX_DURATION_SECS: u64 = 3600;

/// Resolve the orchestrator VM wall-clock budget from
/// `IRONCLAW_ORCHESTRATOR_MAX_DURATION_SECS`. Cached for the process lifetime.
fn orchestrator_max_duration() -> std::time::Duration {
    static CACHED: OnceLock<std::time::Duration> = OnceLock::new();
    *CACHED.get_or_init(|| {
        let secs = std::env::var("IRONCLAW_ORCHESTRATOR_MAX_DURATION_SECS")
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(ORCHESTRATOR_DEFAULT_MAX_DURATION_SECS)
            .clamp(
                ORCHESTRATOR_MIN_MAX_DURATION_SECS,
                ORCHESTRATOR_MAX_MAX_DURATION_SECS,
            );
        std::time::Duration::from_secs(secs)
    })
}

/// Resource limits for the orchestrator VM.
fn orchestrator_limits() -> ResourceLimits {
    ResourceLimits::new()
        .max_duration(orchestrator_max_duration())
        .max_allocations(5_000_000)
        .max_memory(128 * 1024 * 1024) // 128 MB
}

/// Classify a Monty orchestrator failure into a typed
/// [`OrchestratorFailure`] that carries a user-safe classification plus
/// the preserved low-level detail for gateway debug mode.
///
/// The raw `err_msg` (often a Python traceback containing internal file
/// paths and upstream HTTP bodies) is always stored on the returned
/// struct's `debug_detail` field and emitted at `debug!`, never placed
/// into the user-visible classification — see
/// `.claude/rules/error-handling.md`, "Error Boundaries at the Channel
/// Edge" (#2546).
fn classify_orchestrator_failure(prefix: &str, err_msg: &str) -> OrchestratorFailure {
    debug!(prefix, err_msg, "orchestrator VM failure");

    let lower = err_msg.to_ascii_lowercase();
    // Reserve `TimeLimit` for unmistakable Monty wall-clock markers — the
    // user-facing message tells operators to raise
    // `IRONCLAW_ORCHESTRATOR_MAX_DURATION_SECS`, which is wrong advice for
    // upstream LLM / network timeouts. Bare `"timeout"` / `"timed out"`
    // used to catch those (e.g. `reqwest`'s `"Request timed out"`,
    // provider `"Connection timed out"`) and point users at the budget
    // knob instead of the real failure class. Those now fall through to
    // `Other` (generic internal failure). References: serrrfirat review
    // on PR #2753, commit 82d06410.
    //
    // The predicates we keep are either the explicit env-var name in the
    // VM's own error text, the phrase the Monty runtime uses for its
    // duration limit, or the sentinel emitted by the engine when the
    // orchestrator itself times out a step. Duplicating `ResourceLimits`
    // wording is OK — those strings live alongside this classifier in the
    // same crate.
    let hit_time_limit = lower.contains("duration limit")
        || lower.contains("max_duration")
        || lower.contains("maximum duration")
        || lower.contains("execution duration exceeded")
        || lower.contains("orchestrator timed out");
    let hit_memory_limit = lower.contains("memory limit") || lower.contains("allocation limit");
    let hit_resource_limit = lower.contains("resource limit")
        || lower.contains("out of fuel")
        || lower.contains("fuel exhausted");
    let has_python_traceback =
        lower.contains("traceback (most recent call last)") || lower.contains("traceback:");

    let kind = if hit_time_limit {
        OrchestratorFailureKind::TimeLimit {
            prefix: prefix.to_string(),
            limit_secs: orchestrator_max_duration().as_secs(),
        }
    } else if hit_memory_limit || hit_resource_limit {
        OrchestratorFailureKind::ResourceLimit {
            prefix: prefix.to_string(),
        }
    } else if has_python_traceback {
        OrchestratorFailureKind::Traceback {
            prefix: prefix.to_string(),
        }
    } else {
        OrchestratorFailureKind::Other {
            prefix: prefix.to_string(),
        }
    };

    OrchestratorFailure::new(kind, err_msg)
}

/// Wrap a Monty VM panic (parse / start / resume phase) as a typed
/// orchestrator failure. The panic itself has no textual payload — the
/// `panic_payload` we can stringify is always a `&str` or `String` from
/// `catch_unwind` — so `debug_detail` carries the phase tag for
/// correlation.
fn orchestrator_vm_panic(prefix: &str, phase: &'static str) -> OrchestratorFailure {
    debug!(prefix, phase, "orchestrator VM panic");
    OrchestratorFailure::new(
        OrchestratorFailureKind::VmPanic {
            prefix: prefix.to_string(),
            phase,
        },
        format!("Monty VM panicked during {phase}"),
    )
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

    // The failure tracker carries the `orchestrator:` title prefix and is
    // therefore gated by `is_protected_orchestrator_doc` in the store.
    // Enter the trusted-internal-writes scope so the system-initiated save
    // is admitted without being mistaken for an LLM-authored patch.
    if let Err(e) =
        crate::runtime::with_trusted_internal_writes(store.save_memory_doc(&tracker)).await
    {
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
        // Same rationale as `record_orchestrator_failure`: the tracker doc
        // has an `orchestrator:` title so the store gate triggers. Enter
        // the trusted-writes scope for this system-initiated reset.
        let _ = crate::runtime::with_trusted_internal_writes(store.save_memory_doc(&tracker)).await;
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
    platform_info: Option<&crate::executor::prompt::PlatformInfo>,
    gate_controller: &Arc<dyn crate::gate::GateController>,
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
            // Route parse failures through the same typed sanitizer so
            // a bad `default.py` deploy can't leak Monty internals to
            // the channel edge.
            return Err(EngineError::Orchestrator(classify_orchestrator_failure(
                "Orchestrator parse error",
                &e.to_string(),
            )));
        }
        Err(_) => {
            return Err(EngineError::Orchestrator(orchestrator_vm_panic(
                "Orchestrator parse error",
                "orchestrator parsing",
            )));
        }
    };

    // Start execution
    let mut stdout = String::new();
    let tracker = LimitedTracker::new(orchestrator_limits());

    let run_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        runner.start(
            input_values,
            tracker,
            PrintWriter::CollectString(&mut stdout),
        )
    }));

    let mut progress = match run_result {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            return Err(EngineError::Orchestrator(classify_orchestrator_failure(
                "Orchestrator runtime error",
                &e.to_string(),
            )));
        }
        Err(_) => {
            return Err(EngineError::Orchestrator(orchestrator_vm_panic(
                "Orchestrator runtime error",
                "orchestrator start",
            )));
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
                                platform_info,
                            },
                            &mut total_tokens,
                        )
                        .await
                    }

                    // __execute_code_step__(code, state)
                    "__execute_code_step__" => {
                        handle_execute_code_step(
                            args,
                            kwargs,
                            thread,
                            llm,
                            effects,
                            leases,
                            policy,
                            event_tx,
                            gate_controller,
                        )
                        .await
                    }

                    // __execute_action__(name, params, call_id=...)
                    "__execute_action__" => {
                        handle_execute_action(
                            args,
                            kwargs,
                            thread,
                            effects,
                            leases,
                            policy,
                            event_tx,
                            gate_controller,
                        )
                        .await
                    }

                    // __execute_actions_parallel__(calls)
                    "__execute_actions_parallel__" => {
                        handle_execute_actions_parallel(
                            args,
                            thread,
                            effects,
                            leases,
                            policy,
                            event_tx,
                            gate_controller,
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
                    call.resume(ext_result, PrintWriter::CollectString(&mut stdout))
                })) {
                    Ok(Ok(p)) => progress = p,
                    Ok(Err(e)) => {
                        return Err(EngineError::Orchestrator(classify_orchestrator_failure(
                            "Orchestrator error after resume",
                            &e.to_string(),
                        )));
                    }
                    Err(_) => {
                        return Err(EngineError::Orchestrator(orchestrator_vm_panic(
                            "Orchestrator error after resume",
                            "orchestrator resume",
                        )));
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
                        PrintWriter::CollectString(&mut stdout),
                    )
                })) {
                    Ok(Ok(p)) => progress = p,
                    Ok(Err(e)) => {
                        return Err(EngineError::Orchestrator(classify_orchestrator_failure(
                            &format!("Orchestrator NameError '{name}'"),
                            &e.to_string(),
                        )));
                    }
                    Err(_) => {
                        return Err(EngineError::Orchestrator(orchestrator_vm_panic(
                            &format!("Orchestrator NameError '{name}'"),
                            "name lookup",
                        )));
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
    platform_info: Option<&'a crate::executor::prompt::PlatformInfo>,
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
    let mut messages = explicit_messages
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
    // Read-only path: `available_actions` and the message refresh below
    // don't pause; inert controller is correct.
    let actions_context = thread_execution_context(
        thread,
        StepId::new(),
        None,
        crate::gate::CancellingGateController::arc(),
    );
    let actions = deps
        .effects
        .available_actions(&active_leases, &actions_context)
        .await
        .unwrap_or_default();
    refresh_llm_messages_for_current_surface(
        &mut messages,
        thread,
        deps.effects,
        deps.store,
        deps.platform_info,
        &active_leases,
        &actions_context,
        &actions,
    )
    .await;

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
        model: explicit_config
            .as_ref()
            .and_then(|cfg| cfg.get("model"))
            .and_then(|v| v.as_str())
            .map(String::from),
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

#[allow(clippy::too_many_arguments)]
async fn refresh_llm_messages_for_current_surface(
    messages: &mut Vec<ThreadMessage>,
    thread: &Thread,
    effects: &Arc<dyn EffectExecutor>,
    store: Option<&Arc<dyn Store>>,
    platform_info: Option<&crate::executor::prompt::PlatformInfo>,
    active_leases: &[crate::types::capability::CapabilityLease],
    actions_context: &ThreadExecutionContext,
    actions: &[crate::types::capability::ActionDef],
) {
    if !messages.iter().any(|message| {
        message.role == crate::types::message::MessageRole::System
            && crate::executor::prompt::is_codeact_system_prompt(&message.content)
    }) {
        return;
    }

    let capabilities = match effects
        .available_capabilities(active_leases, actions_context)
        .await
    {
        Ok(capabilities) => capabilities,
        Err(error) => {
            debug!(
                thread_id = %thread.id,
                "failed to load capabilities for llm_complete prompt refresh: {error}"
            );
            Vec::new()
        }
    };

    let system_prompt = crate::executor::prompt::build_codeact_system_prompt(
        &capabilities,
        actions,
        store,
        thread.project_id,
        platform_info,
    )
    .await;

    crate::executor::prompt::upsert_codeact_system_prompt(messages, system_prompt);
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
    gate_controller: &Arc<dyn crate::gate::GateController>,
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

    let exec_ctx = thread_execution_context(thread, StepId::new(), None, gate_controller.clone());

    // Run user code in a nested Monty VM (same pattern as rlm_query)
    let code_start = std::time::Instant::now();
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
            if let Some(ref category) = result.failure {
                let error_msg = if !result.stdout.is_empty() {
                    format!(
                        "CodeAct execution failed: {}",
                        tail_chars(&result.stdout, 500)
                    )
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
                        duration_ms: code_start.elapsed().as_millis() as u64,
                        params_summary: None,
                    },
                );
                if let Some(tx) = event_tx {
                    let _ = tx.send(failed_event.clone());
                }
                thread.events.push(failed_event);

                // Emit structured CodeExecutionFailed event for instrumentation.
                // This enables aggregate analysis of WHY code execution fails
                // (Monty limitation vs LLM logic error vs tool dispatch failure).
                let error_text = tail_chars(&result.stdout, 500);
                let instrumentation_event = ThreadEvent::new(
                    thread.id,
                    EventKind::CodeExecutionFailed {
                        step_id: exec_ctx.step_id,
                        category: category.clone(),
                        error: error_text,
                        code_hash: Some(crate::executor::scripting::code_hash(&code)),
                        duration_ms: code_start.elapsed().as_millis() as u64,
                    },
                );
                if let Some(tx) = event_tx {
                    let _ = tx.send(instrumentation_event.clone());
                }
                thread.events.push(instrumentation_event);
            }

            // Always emit CodeExecuted so debug observers see the exact code
            // and stdout, regardless of success/failure. The in-context chat
            // summary is too lossy for diagnostics. Kept separate from
            // CodeExecutionFailed (which carries the failure classifier) and
            // the per-action events already broadcast above.
            //
            // Cap code, stdout, and return_value at `CODE_EXECUTED_MAX_BYTES`
            // each before emission so a step that prints or returns a large
            // blob cannot bloat persisted thread events or flood the SSE
            // broadcast buffer. Byte-based (not `chars().count()`) to keep
            // this O(1) even for very large payloads. `tail_chars` elsewhere
            // in this file is left alone because its callers already run
            // inside pre-bounded inputs (`OUTPUT_TRUNCATE_LEN`, 500-char
            // error slices) where chars-vs-bytes is not a perf concern.
            const CODE_EXECUTED_MAX_BYTES: usize = 8_000;
            let code_executed_event = ThreadEvent::new(
                thread.id,
                EventKind::CodeExecuted {
                    step_id: exec_ctx.step_id,
                    code: tail_utf8_bytes(&code, CODE_EXECUTED_MAX_BYTES),
                    stdout: tail_utf8_bytes(&result.stdout, CODE_EXECUTED_MAX_BYTES),
                    return_value: bounded_return_value(
                        &result.return_value,
                        CODE_EXECUTED_MAX_BYTES,
                    ),
                    duration_ms: code_start.elapsed().as_millis() as u64,
                },
            );
            if let Some(tx) = event_tx {
                let _ = tx.send(code_executed_event.clone());
            }
            thread.events.push(code_executed_event);

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
                "had_error": result.failure.is_some(),
                "pending_gate": result.need_approval.as_ref().map(|na| {
                    match na {
                        ThreadOutcome::GatePaused { gate_name, action_name, call_id, parameters, resume_kind, resume_output, paused_lease } => serde_json::json!({
                            "gate_paused": true,
                            "gate_name": gate_name,
                            "action_name": action_name,
                            "call_id": call_id,
                            "parameters": parameters,
                            "resume_kind": serde_json::to_value(resume_kind).unwrap_or_default(),
                            "resume_output": resume_output,
                            "paused_lease": paused_lease,
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
#[allow(clippy::too_many_arguments)]
async fn handle_execute_action(
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
    thread: &mut Thread,
    effects: &Arc<dyn EffectExecutor>,
    leases: &Arc<LeaseManager>,
    policy: &Arc<PolicyEngine>,
    event_tx: Option<&tokio::sync::broadcast::Sender<ThreadEvent>>,
    gate_controller: &Arc<dyn crate::gate::GateController>,
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

    let mut exec_ctx = thread_execution_context(
        thread,
        StepId::new(),
        Some(call_id.clone()),
        gate_controller.clone(),
    );
    let active_leases = leases.active_for_thread(thread.id).await;
    let inventory = match effects
        .available_action_inventory(&active_leases, &exec_ctx)
        .await
    {
        Ok(inventory) => Some(Arc::new(inventory)),
        Err(error) => {
            debug!(
                thread_id = %thread.id,
                action = %name,
                "failed to load action inventory for orchestrator action execution: {error}"
            );
            None
        }
    };
    let available_actions = apply_snapshot_inventory(&mut exec_ctx, inventory);

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

    // 1. Find the action definition from the callable inventory.
    let action_def = available_actions.iter().find(|a| a.matches_name(&name));
    if exec_ctx.available_actions_snapshot.is_some() && action_def.is_none() {
        let error = format!("action '{name}' is not callable in this execution context");
        let output = serde_json::json!({"error": &error});
        emit_and_record(
            thread,
            event_tx,
            EventKind::ActionFailed {
                step_id: exec_ctx.step_id,
                action_name: name.clone(),
                call_id: call_id.clone(),
                error,
                duration_ms: 0,
                params_summary: summarize_params(&name, &params),
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

    // 2. Find lease for this action
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
                    duration_ms: 0,
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

    let canonical_name = action_def
        .as_ref()
        .map(|action| action.name.clone())
        .unwrap_or_else(|| name.clone());

    if let Some(ad) = action_def {
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
                        duration_ms: 0,
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
                // Inline gate-await on policy-raised approval. Mirrors
                // `structured.rs::execute_action_batch_with_results`: emit
                // the request, pause the executor in place, and either
                // fall through to lease consume + execute on approval, or
                // emit ActionFailed and surface a deny-style result on
                // denial. No more `gate_paused` sentinel + thread re-entry
                // for this code path.
                emit_and_record(
                    thread,
                    event_tx,
                    EventKind::ApprovalRequested {
                        action_name: name.clone(),
                        call_id: call_id.clone(),
                        parameters: Some(params.clone()),
                        description: None,
                        allow_always: Some(true),
                        gate_name: Some("approval".into()),
                        params_summary: summarize_params(&name, &params),
                    },
                    &call_id,
                    &name,
                    &serde_json::json!({}),
                );

                let resume_kind = crate::gate::ResumeKind::Approval { allow_always: true };
                let resolution = gate_controller
                    .pause(crate::gate::GatePauseRequest {
                        thread_id: thread.id,
                        user_id: thread.user_id.clone(),
                        gate_name: "approval".into(),
                        action_name: name.clone(),
                        call_id: call_id.clone(),
                        parameters: params.clone(),
                        resume_kind,
                        conversation_id: exec_ctx.conversation_id,
                    })
                    .await;

                if let Some(outcome) =
                    crate::executor::scripting::denial_outcome_for_resolution(&resolution)
                {
                    let error = outcome.event_error();
                    let output = serde_json::json!({"error": &error});
                    emit_and_record(
                        thread,
                        event_tx,
                        EventKind::ActionFailed {
                            step_id: exec_ctx.step_id,
                            action_name: name.clone(),
                            call_id: call_id.clone(),
                            error: error.clone(),
                            duration_ms: 0,
                            params_summary: summarize_params(&name, &params),
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
                // Approved — fall through to lease consume + execute.
                // The adapter's per-call ApprovalRequirement gate (if
                // any) is independent of the policy gate and will be
                // handled inline by the wrapper below if it fires.
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
                    duration_ms: 0,
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

    // 4. Execute via the inline-await wrapper. Tool-raised
    // `Err(GatePaused)` from `effects.execute_action` is converted to a
    // `gate_paused` JSON sentinel by the adapter shim and then handled
    // inline by `execute_single_action_with_inline_retry`: pause the
    // user, retry on approval (bounded), surface deny-style results
    // on denial. No more `gate_paused` sentinel returned to Python
    // from this path.
    let ps = summarize_params(&canonical_name, &params);
    let (result_json, events, _output, _final_lease_id) = execute_single_action_with_inline_retry(
        effects,
        leases,
        &canonical_name,
        params,
        &call_id,
        lease,
        &exec_ctx,
        ps,
        thread.id,
        &thread.user_id,
    )
    .await;
    for event in events {
        emit_and_record(
            thread,
            event_tx,
            event,
            &call_id,
            &name,
            &serde_json::json!({}),
        );
    }
    ExtFunctionResult::Return(json_to_monty(&result_json))
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
#[allow(clippy::too_many_arguments)]
async fn handle_execute_actions_parallel(
    args: &[MontyObject],
    thread: &mut Thread,
    effects: &Arc<dyn EffectExecutor>,
    leases: &Arc<LeaseManager>,
    policy: &Arc<PolicyEngine>,
    event_tx: Option<&tokio::sync::broadcast::Sender<ThreadEvent>>,
    gate_controller: &Arc<dyn crate::gate::GateController>,
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
    let actions_context = thread_execution_context(thread, step_id, None, gate_controller.clone());
    let active_leases = leases.active_for_thread(thread.id).await;
    let inventory = match effects
        .available_action_inventory(&active_leases, &actions_context)
        .await
    {
        Ok(inventory) => Some(Arc::new(inventory)),
        Err(error) => {
            debug!(
                thread_id = %thread.id,
                step_id = ?step_id,
                "failed to load action inventory for orchestrator parallel execution: {error}"
            );
            None
        }
    };
    let available_actions: Arc<[crate::types::capability::ActionDef]> = inventory
        .as_ref()
        .map(|inventory| inventory.inline.clone().into())
        .unwrap_or_else(|| Arc::from([]));

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
        // Find the action definition from the callable inventory.
        let mut exec_ctx = thread_execution_context(
            thread,
            step_id,
            Some(pc.call_id.clone()),
            gate_controller.clone(),
        );
        if let Some(ref inventory) = inventory {
            exec_ctx.available_actions_snapshot = Some(Arc::clone(&available_actions));
            exec_ctx.available_action_inventory_snapshot = Some(Arc::clone(inventory));
        }
        let action_def = available_actions
            .iter()
            .find(|a| a.matches_name(&pc.name))
            .cloned();
        if inventory.is_some() && action_def.is_none() {
            let error = format!(
                "action '{}' is not callable in this execution context",
                pc.name
            );
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
                duration_ms: 0,
                params_summary: summarize_params(&pc.name, &pc.params),
            };
            preflight.push(Some(PfOutcome::Error {
                result_json,
                event,
                output,
            }));
            continue;
        }

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
                    duration_ms: 0,
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
        let action_name = action_def
            .as_ref()
            .map(|action| action.name.clone())
            .unwrap_or_else(|| pc.name.clone());

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
                        action_name: action_name.clone(),
                        call_id: pc.call_id.clone(),
                        error: reason,
                        duration_ms: 0,
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
                    // Inline gate-await: pause this preflight call in place
                    // until the user resolves the gate. On approval, fall
                    // through to lease consumption + queue for execution.
                    // On denial, push an ActionFailed result and continue
                    // preflight so the rest of the batch still runs —
                    // mirrors `structured.rs::execute_action_batch_with_results`.
                    //
                    // The bridge controller serializes concurrent inline
                    // gates per (user, thread), so two preflight calls that
                    // both gate get prompted sequentially rather than the
                    // second silently cancelling.
                    let approval_ev = ThreadEvent::new(
                        thread.id,
                        EventKind::ApprovalRequested {
                            action_name: pc.name.clone(),
                            call_id: pc.call_id.clone(),
                            parameters: Some(pc.params.clone()),
                            description: None,
                            allow_always: Some(true),
                            gate_name: Some("approval".into()),
                            params_summary: summarize_params(&pc.name, &pc.params),
                        },
                    );
                    if let Some(tx) = event_tx {
                        let _ = tx.send(approval_ev.clone());
                    }
                    thread.events.push(approval_ev);
                    thread.updated_at = chrono::Utc::now();

                    let resume_kind = crate::gate::ResumeKind::Approval { allow_always: true };
                    let resolution = gate_controller
                        .pause(crate::gate::GatePauseRequest {
                            thread_id: thread.id,
                            user_id: thread.user_id.clone(),
                            gate_name: "approval".into(),
                            action_name: pc.name.clone(),
                            call_id: pc.call_id.clone(),
                            parameters: pc.params.clone(),
                            resume_kind,
                            conversation_id: exec_ctx.conversation_id,
                        })
                        .await;

                    if let Some(outcome) =
                        crate::executor::scripting::denial_outcome_for_resolution(&resolution)
                    {
                        let error = outcome.event_error();
                        let output = serde_json::json!({"error": &error});
                        let result_json = serde_json::json!({
                            "output": &output,
                            "is_error": true,
                        });
                        let event = EventKind::ActionFailed {
                            step_id,
                            action_name: action_name.clone(),
                            call_id: pc.call_id.clone(),
                            error,
                            duration_ms: 0,
                            params_summary: summarize_params(&pc.name, &pc.params),
                        };
                        preflight.push(Some(PfOutcome::Error {
                            result_json,
                            event,
                            output,
                        }));
                        continue;
                    }
                    // Approved — fall through to lease consume + runnable.
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
                    duration_ms: 0,
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

    // Slot array: index → execution result. `slot_events` is
    // `Vec<EventKind>` per slot so the inline-retry path can record
    // multiple events (ApprovalRequested + post-retry outcome).
    let mut slot_results: Vec<Option<serde_json::Value>> = vec![None; parsed.len()];
    let mut slot_events: Vec<Option<Vec<EventKind>>> = vec![None; parsed.len()];
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
                slot_events[idx] = Some(vec![event]);
                slot_outputs[idx] = Some(output);
            }
            Some(PfOutcome::Runnable { lease }) => {
                runnable.push((idx, lease));
            }
            None => {}
        }
    }

    if runnable.len() == 1 {
        // Single call: execute directly with inline gate-await retry.
        let (idx, lease) = runnable.into_iter().next().unwrap(); // safety: len()==1 checked above
        let pc = &parsed[idx];
        let action_name = available_actions
            .iter()
            .find(|action| action.matches_name(&pc.name))
            .map(|action| action.name.clone())
            .unwrap_or_else(|| pc.name.clone());
        let mut exec_ctx = thread_execution_context(
            thread,
            step_id,
            Some(pc.call_id.clone()),
            gate_controller.clone(),
        );
        if let Some(ref inventory) = inventory {
            exec_ctx.available_actions_snapshot = Some(Arc::clone(&available_actions));
            exec_ctx.available_action_inventory_snapshot = Some(Arc::clone(inventory));
        }
        let ps = summarize_params(&action_name, &pc.params);
        let (result_json, events, output, _final_lease_id) =
            execute_single_action_with_inline_retry(
                effects,
                leases,
                &action_name,
                pc.params.clone(),
                &pc.call_id,
                lease,
                &exec_ctx,
                ps,
                thread.id,
                &thread.user_id,
            )
            .await;
        slot_results[idx] = Some(result_json);
        slot_events[idx] = Some(events);
        slot_outputs[idx] = Some(output);
    } else if runnable.len() > 1 {
        // Multiple calls: execute in parallel via JoinSet. Each task
        // carries its own inline retry loop so one tool's gate doesn't
        // block the rest of the batch — and the legacy "double-execute
        // on resume" bug never fires for parallel batches either.
        let mut join_set = tokio::task::JoinSet::new();
        let effects = effects.clone();
        let leases_arc = Arc::clone(leases);
        // Build the base execution context once from the live thread.
        // Per-task contexts clone this and overwrite `current_call_id`
        // and the action snapshots — far cheaper than cloning the full
        // `Thread` (which carries message/event transcripts) per task.
        let base_exec_ctx =
            thread_execution_context(thread, step_id, None, gate_controller.clone());
        let thread_id = thread.id;
        let user_id = thread.user_id.clone();
        for (idx, lease) in runnable {
            let pc_name = available_actions
                .iter()
                .find(|action| action.matches_name(&parsed[idx].name))
                .map(|action| action.name.clone())
                .unwrap_or_else(|| parsed[idx].name.clone());
            let pc_params = parsed[idx].params.clone();
            let pc_call_id = parsed[idx].call_id.clone();
            let effects = effects.clone();
            let leases = Arc::clone(&leases_arc);
            let user_id = user_id.clone();
            let lease = lease.clone();
            let mut exec_ctx = base_exec_ctx.clone();
            exec_ctx.current_call_id = Some(pc_call_id.clone());
            if let Some(ref inventory) = inventory {
                exec_ctx.available_actions_snapshot = Some(Arc::clone(&available_actions));
                exec_ctx.available_action_inventory_snapshot = Some(Arc::clone(inventory));
            }
            let ps = summarize_params(&pc_name, &pc_params);

            join_set.spawn(async move {
                let (result_json, events, output, final_lease_id) =
                    execute_single_action_with_inline_retry(
                        &effects,
                        &leases,
                        &pc_name,
                        pc_params,
                        &pc_call_id,
                        lease,
                        &exec_ctx,
                        ps,
                        thread_id,
                        &user_id,
                    )
                    .await;
                (idx, final_lease_id, result_json, events, output)
            });
        }

        while let Some(join_result) = join_set.join_next().await {
            match join_result {
                Ok((idx, _lease_id, result_json, events, output)) => {
                    // The inline-retry helper already refunded any
                    // leases consumed during gate-await. No
                    // additional bookkeeping needed here.
                    slot_results[idx] = Some(result_json);
                    slot_events[idx] = Some(events);
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

        if let Some(events) = slot_events[idx].take() {
            for event in events {
                let ev = ThreadEvent::new(thread.id, event);
                if let Some(tx) = event_tx {
                    let _ = tx.send(ev.clone());
                }
                thread.events.push(ev);
            }
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
    let execution_start = std::time::Instant::now();
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
                let duration_ms = r.duration.as_millis() as u64;
                EventKind::ActionFailed {
                    step_id: exec_ctx.step_id,
                    action_name: name.to_string(),
                    call_id: call_id.to_string(),
                    error: error_msg,
                    duration_ms: if duration_ms > 0 {
                        duration_ms
                    } else {
                        execution_start.elapsed().as_millis() as u64
                    },
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
            paused_lease,
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
                "paused_lease": paused_lease.as_deref().cloned(),
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
                duration_ms: execution_start.elapsed().as_millis() as u64,
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

/// Like [`execute_single_action`] but pauses inline on
/// `Approval`-kind gate paused results and retries. Bounded by
/// [`crate::executor::scripting::MAX_INLINE_GATE_RETRIES`] so a
/// misbehaving tool can't spin a CPU.
///
/// Used by `__execute_actions_parallel__` for both the single-runnable
/// and multi-runnable branches. Without this wrapper the multi-runnable
/// branch falls through to the legacy `gate_paused` sentinel + thread
/// re-entry, which double-executes earlier non-idempotent calls in the
/// same batch — exactly the bug this PR exists to prevent.
#[allow(clippy::too_many_arguments)]
async fn execute_single_action_with_inline_retry(
    effects: &Arc<dyn EffectExecutor>,
    leases: &Arc<LeaseManager>,
    name: &str,
    params: serde_json::Value,
    call_id: &str,
    initial_lease: crate::types::capability::CapabilityLease,
    exec_ctx: &ThreadExecutionContext,
    params_summary: Option<String>,
    thread_id: crate::types::thread::ThreadId,
    user_id: &str,
) -> (
    serde_json::Value,
    Vec<EventKind>,
    serde_json::Value,
    crate::types::capability::LeaseId,
) {
    let mut current_lease = initial_lease;
    let mut call_ctx = exec_ctx.clone();
    // `accumulated_events` carries every event the inline-retry loop
    // observes — `ApprovalRequested` from each gate-paused iteration,
    // plus the final `ActionExecuted` / `ActionFailed`. The caller
    // appends them all to the thread event log so observers see the
    // full sequence.
    let mut accumulated_events: Vec<EventKind> = Vec::new();
    for _ in 0..crate::executor::scripting::MAX_INLINE_GATE_RETRIES {
        let (result_json, event, output) = execute_single_action(
            effects,
            name,
            params.clone(),
            call_id,
            &current_lease,
            &call_ctx,
            params_summary.clone(),
        )
        .await;
        // Reset the one-shot approval flag — only the call immediately
        // following an approval should carry it.
        call_ctx.call_approval_granted = false;

        if !interrupted_result_needs_refund(&result_json) {
            // Not a gate pause — terminal event; record it and return.
            accumulated_events.push(event);
            return (result_json, accumulated_events, output, current_lease.id);
        }

        // Gate paused. Approval and Authentication get the inline-await
        // treatment (#3133 / #3166): host controller resolves them in
        // place, the suspended call retries, and the orchestrator
        // continues without unwinding. External keeps the legacy
        // `gate_paused` sentinel + re-entry path because its resolution
        // payload (callback body) can't be handed back to a suspended
        // call.
        let resume_kind: crate::gate::ResumeKind = result_json
            .get("resume_kind")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or(crate::gate::ResumeKind::Approval {
                allow_always: false,
            });
        if !matches!(
            resume_kind,
            crate::gate::ResumeKind::Approval { .. }
                | crate::gate::ResumeKind::Authentication { .. }
        ) {
            accumulated_events.push(event);
            return (result_json, accumulated_events, output, current_lease.id);
        }

        // Approval gate fired — record the request before pausing the
        // controller so observers see the prompt regardless of how the
        // resolution lands.
        accumulated_events.push(event);

        // Refund the lease use this attempt consumed; we'll re-consume
        // on retry if the user approves.
        let _ = leases.refund_use(current_lease.id).await;

        // Use the gate-provided parameters from the GatePaused payload,
        // not the original caller `params`: the safety layer may have
        // transformed/redacted them, and the prompt the user sees must
        // match what the tool actually wanted to run with. Mirrors the
        // contract in `structured::execute_with_inline_gate_retry`.
        let gate_parameters = result_json
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| params.clone());
        let resolution = exec_ctx
            .gate_controller
            .pause(crate::gate::GatePauseRequest {
                thread_id,
                user_id: user_id.to_string(),
                gate_name: result_json
                    .get("gate_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("approval")
                    .to_string(),
                action_name: name.to_string(),
                call_id: call_id.to_string(),
                parameters: gate_parameters,
                resume_kind: resume_kind.clone(),
                conversation_id: exec_ctx.conversation_id,
            })
            .await;

        if let Some(outcome) =
            crate::executor::scripting::denial_outcome_for_resolution(&resolution)
        {
            // Cancelled+Authentication → fall through to legacy
            // `gate_paused` sentinel so missions / non-inline-aware
            // controllers can still surface a Paused state. See the
            // matching branch in
            // `structured::execute_with_inline_gate_retry`. The
            // already-accumulated `ApprovalRequested` event was
            // pushed before the pause; we re-emit it on the new
            // result_json carrying the original gate metadata.
            if matches!(resolution, crate::gate::GateResolution::Cancelled)
                && matches!(resume_kind, crate::gate::ResumeKind::Authentication { .. })
            {
                return (result_json, accumulated_events, output, current_lease.id);
            }
            let error_msg = outcome.event_error();
            let denial = serde_json::json!({"error": &error_msg});
            let denial_event = EventKind::ActionFailed {
                step_id: exec_ctx.step_id,
                action_name: name.to_string(),
                call_id: call_id.to_string(),
                error: error_msg,
                duration_ms: 0,
                params_summary: params_summary.clone(),
            };
            accumulated_events.push(denial_event);
            let result_json = serde_json::json!({
                "action_name": name,
                "output": &denial,
                "is_error": true,
                "duration_ms": 0,
            });
            return (result_json, accumulated_events, denial, current_lease.id);
        }

        // Approved. Re-consume a lease use and mark the next call as
        // pre-approved.
        match leases.find_and_consume(thread_id, name).await {
            Ok(new_lease) => {
                current_lease = new_lease;
                call_ctx.call_approval_granted = true;
                continue;
            }
            Err(e) => {
                let err =
                    serde_json::json!({"error": format!("lease exhausted after approval: {e}")});
                let lease_event = EventKind::ActionFailed {
                    step_id: exec_ctx.step_id,
                    action_name: name.to_string(),
                    call_id: call_id.to_string(),
                    error: format!("lease exhausted after approval: {e}"),
                    duration_ms: 0,
                    params_summary: params_summary.clone(),
                };
                accumulated_events.push(lease_event);
                let result_json = serde_json::json!({
                    "action_name": name,
                    "output": &err,
                    "is_error": true,
                    "duration_ms": 0,
                });
                return (result_json, accumulated_events, err, current_lease.id);
            }
        }
    }

    // Retry budget exhausted — tool kept gating after every approval.
    // The last loop iteration ended with a successful `find_and_consume`
    // whose lease was never used; refund it before returning so a
    // misbehaving tool can't slowly drain `max_uses` across approvals.
    // Best-effort; if the lease was already revoked/expired the refund
    // is a no-op.
    let _ = leases.refund_use(current_lease.id).await;
    let err = serde_json::json!({
        "error": format!(
            "tool '{name}' still requires approval after {} retries",
            crate::executor::scripting::MAX_INLINE_GATE_RETRIES
        ),
    });
    accumulated_events.push(EventKind::ActionFailed {
        step_id: exec_ctx.step_id,
        action_name: name.to_string(),
        call_id: call_id.to_string(),
        error: format!(
            "tool kept gating after {} approvals",
            crate::executor::scripting::MAX_INLINE_GATE_RETRIES
        ),
        duration_ms: 0,
        params_summary,
    });
    let result_json = serde_json::json!({
        "action_name": name,
        "output": &err,
        "is_error": true,
        "duration_ms": 0,
    });
    (result_json, accumulated_events, err, current_lease.id)
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
            let duration_ms = extract_u64_kwarg(kwargs, "duration_ms").unwrap_or(0);
            EventKind::ActionFailed {
                step_id: StepId::new(),
                action_name,
                call_id,
                error,
                duration_ms,
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
                "consecutive_action_errors": counters.get("consecutive_action_errors").and_then(|v| v.as_u64()).unwrap_or(0),
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
    // Read-only path: `available_actions` doesn't pause, so an inert
    // controller is correct. Plumbing the live one here would buy
    // nothing.
    let actions_context = thread_execution_context(
        thread,
        StepId::new(),
        None,
        crate::gate::CancellingGateController::arc(),
    );
    match effects
        .available_actions(&active_leases, &actions_context)
        .await
    {
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
///
/// ## Setup-marker exclusion (v2 parity with v1 selector)
///
/// Before returning the skill list, this function filters out any
/// skill whose `metadata.activation.setup_marker` is already present
/// as a MemoryDoc title in the current project. In v2, workspace
/// files are stored as MemoryDocs keyed by title, so "does the marker
/// file exist" maps to "is there a MemoryDoc with that title" — and
/// we already have the full doc list in scope for the skill filter,
/// so this costs zero extra store calls.
///
/// This is the v2 equivalent of the `satisfied_setup_markers`
/// argument threaded through `ironclaw_skills::prefilter_skills` on
/// the v1 path. Both paths implement the same rule: a one-time setup
/// skill whose marker file has been written has finished its job and
/// should not keep burning activation budget on every subsequent turn.
async fn handle_list_skills(
    _args: &[MontyObject],
    thread: &Thread,
    store: Option<&Arc<dyn Store>>,
) -> ExtFunctionResult {
    let Some(store) = store else {
        return ExtFunctionResult::Return(json_to_monty(&serde_json::json!([])));
    };

    // User's docs in their project (all doc types — skill filtering happens
    // below in the `filter(|d| d.doc_type == Skill)` pass).
    let mut docs = match store
        .list_memory_docs(thread.project_id, &thread.user_id)
        .await
    {
        Ok(d) => d,
        Err(e) => {
            debug!("__list_skills__: failed to load user docs: {e}");
            vec![]
        }
    };

    // Admin/shared skills across ALL projects (fixes multi-tenant visibility —
    // shared skills live in the owner's project but must be visible to all users
    // regardless of which per-user project their thread runs in).
    match store.list_skills_global().await {
        Ok(shared) => docs.extend(shared),
        Err(e) => debug!("__list_skills__: failed to load global skills: {e}"),
    }

    docs.sort_by_key(|d| d.id.0);
    docs.dedup_by_key(|d| d.id);

    // Build the set of existing non-skill doc titles (== workspace paths
    // in v2) once, so setup-marker filtering below is O(1) per skill.
    // Exclude Skill docs so a marker like "github" doesn't collide with
    // the skill doc of the same name.
    let existing_titles: std::collections::HashSet<&str> = docs
        .iter()
        .filter(|d| d.doc_type != crate::types::memory::DocType::Skill)
        .map(|d| d.title.as_str())
        .collect();

    let skills: Vec<serde_json::Value> = docs
        .iter()
        .filter(|d| d.doc_type == crate::types::memory::DocType::Skill)
        .filter(|d| {
            // Setup-marker exclusion. If the skill's activation
            // metadata declares a setup_marker and a MemoryDoc with
            // that title already exists, the skill's setup has been
            // completed and we skip it.
            let marker = d
                .metadata
                .get("activation")
                .and_then(|a| a.get("setup_marker"))
                .and_then(|m| m.as_str());
            match marker {
                Some(m) if existing_titles.contains(m) => {
                    debug!(
                        skill = %d.title,
                        marker = %m,
                        "__list_skills__: excluding setup skill — marker already present"
                    );
                    false
                }
                _ => true,
            }
        })
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
        "require_action_attempt": thread.config.require_action_attempt,
        "max_action_requirement_nudges": thread.config.max_action_requirement_nudges,
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

/// Extract the last `n` characters from `s`.
///
/// Error tracebacks appear at the end of stdout, after any `print()` output.
/// Using the head would capture the print statements instead of the error.
fn tail_chars(s: &str, n: usize) -> String {
    let char_count = s.chars().count();
    if char_count > n {
        s.chars().skip(char_count - n).collect()
    } else {
        s.to_owned()
    }
}

/// Return the last `max_bytes` bytes of `s`, adjusted forward to the
/// next UTF-8 character boundary so the slice is always valid UTF-8.
///
/// Byte-based (unlike [`tail_chars`]) to stay O(1)+boundary-walk on
/// large payloads. Used by the `CodeExecuted` emission path where
/// `code`/`stdout` can be arbitrarily large, so `chars().count()` on
/// every step would be measurable overhead.
fn tail_utf8_bytes(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_owned();
    }
    let mut start = s.len() - max_bytes;
    // Advance past mid-codepoint bytes. At most 3 iterations because
    // UTF-8 codepoints are ≤ 4 bytes.
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    s[start..].to_owned() // safety: `start` is validated by `is_char_boundary`
}

/// Bound the serialized size of a CodeAct return value before emission.
///
/// - `Null` → `None` (no change — a null return is nothing to surface).
/// - `String` → tail-truncated via [`tail_utf8_bytes`].
/// - Other (`Array`, `Object`, `Number`, `Bool`) → serialized length
///   checked; returned intact if ≤ `max_bytes`, dropped otherwise.
///
/// Dropping (rather than truncating arbitrary JSON) is deliberate: a
/// truncated `Array` / `Object` is unparseable on the frontend and
/// provides no diagnostic value. Observers see a `None` return value
/// and know the payload was omitted for size, not that it was `null`.
fn bounded_return_value(value: &serde_json::Value, max_bytes: usize) -> Option<serde_json::Value> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => {
            Some(serde_json::Value::String(tail_utf8_bytes(s, max_bytes)))
        }
        other => match serde_json::to_vec(other) {
            Ok(buf) if buf.len() <= max_bytes => Some(other.clone()),
            _ => None,
        },
    }
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
            debug_detail: None,
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
                paused_lease: result
                    .get("paused_lease")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok()),
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

    // ── CodeExecuted payload bounding ────────────────────────────

    #[test]
    fn tail_utf8_bytes_returns_input_when_already_small() {
        assert_eq!(tail_utf8_bytes("hello", 100), "hello");
    }

    #[test]
    fn tail_utf8_bytes_tails_ascii_at_byte_boundary() {
        let s: String = "abcdefghij".repeat(100); // 1_000 bytes
        let out = tail_utf8_bytes(&s, 50);
        assert_eq!(out.len(), 50);
        assert!(s.ends_with(&out));
    }

    #[test]
    fn tail_utf8_bytes_advances_past_multibyte_split() {
        // 4-byte emoji followed by ASCII; cut position lands mid-codepoint.
        let s = format!("{}abcde", "🐍".repeat(10)); // 40 bytes of emoji + 5 ASCII = 45 bytes
        let out = tail_utf8_bytes(&s, 9);
        // The walk-forward must have landed on a char boundary; the
        // resulting &str must be valid UTF-8 (implicit — String construction
        // would panic otherwise) and end with the original tail.
        assert!(s.ends_with(&out));
        assert!(out.len() <= 9);
    }

    #[test]
    fn bounded_return_value_drops_null() {
        assert_eq!(bounded_return_value(&serde_json::Value::Null, 100), None);
    }

    #[test]
    fn bounded_return_value_truncates_large_string() {
        let big = "x".repeat(50_000);
        let v = serde_json::Value::String(big);
        let out = bounded_return_value(&v, 100).expect("string retained");
        let s = out.as_str().expect("string variant");
        assert_eq!(s.len(), 100);
    }

    #[test]
    fn bounded_return_value_retains_small_structured() {
        let v = serde_json::json!({"ok": true, "count": 42});
        assert_eq!(bounded_return_value(&v, 1_000), Some(v));
    }

    #[test]
    fn bounded_return_value_drops_oversized_structured() {
        // A 10k-element array serializes well past 8 KiB.
        let items: Vec<_> = (0..10_000).collect();
        let v = serde_json::json!(items);
        assert_eq!(
            bounded_return_value(&v, 8_000),
            None,
            "oversized structured return_value should be dropped, not truncated"
        );
    }

    // ── Orchestrator budget / error mapping ─────────────────────

    #[test]
    fn failure_reason_maps_timeout_to_user_safe_message() {
        let failure = classify_orchestrator_failure(
            "Orchestrator error after resume",
            "ResourceLimits: duration limit exceeded",
        );
        assert!(
            matches!(failure.kind, OrchestratorFailureKind::TimeLimit { .. }),
            "expected TimeLimit variant, got: {:?}",
            failure.kind
        );
        let rendered = failure.user_message();
        assert!(
            rendered.contains("time budget exhausted"),
            "expected user-safe timeout reason, got: {rendered}"
        );
        assert!(
            rendered.contains("IRONCLAW_ORCHESTRATOR_MAX_DURATION_SECS"),
            "reason must point operators at the override env var, got: {rendered}"
        );
    }

    /// Regression for serrrfirat review on PR #2753 (commit 82d06410) —
    /// the classifier used to treat any `"timeout"` / `"timed out"`
    /// substring as a wall-clock exhaustion, so upstream LLM / network
    /// timeouts (`"Request timed out"`, `"Connection timed out"`) were
    /// mapped to `TimeLimit` and the user-facing message advised raising
    /// `IRONCLAW_ORCHESTRATOR_MAX_DURATION_SECS` — completely wrong for a
    /// provider-side timeout. Those now fall through to `Other` so the
    /// budget knob is only suggested when the failure is actually a
    /// Monty wall-clock limit.
    #[test]
    fn failure_reason_does_not_treat_upstream_timeout_as_time_limit() {
        for upstream in [
            "Request timed out",
            "Connection timed out",
            "LLM call failed: timeout waiting for response",
            "upstream provider timeout after 30s",
        ] {
            let failure = classify_orchestrator_failure("Orchestrator runtime error", upstream);
            assert!(
                !matches!(failure.kind, OrchestratorFailureKind::TimeLimit { .. }),
                "upstream timeout {upstream:?} must NOT classify as TimeLimit, got: {:?}",
                failure.kind,
            );
            assert!(
                matches!(failure.kind, OrchestratorFailureKind::Other { .. }),
                "upstream timeout {upstream:?} should fall through to Other, got: {:?}",
                failure.kind,
            );
            let rendered = failure.user_message();
            assert!(
                !rendered.contains("IRONCLAW_ORCHESTRATOR_MAX_DURATION_SECS"),
                "user message for upstream timeout must not advise raising the budget knob, got: {rendered}",
            );
        }
    }

    #[test]
    fn failure_reason_maps_memory_limit() {
        let failure =
            classify_orchestrator_failure("Orchestrator runtime error", "memory limit hit");
        assert!(matches!(
            failure.kind,
            OrchestratorFailureKind::ResourceLimit { .. }
        ));
        assert!(
            failure.user_message().contains("resource budget exhausted"),
            "memory-limit reason should not leak raw Monty text, got: {}",
            failure.user_message()
        );
    }

    /// Regression for Copilot review on PR #2753 (commit 042c2ee7) —
    /// `Other`'s user-facing Display used to embed the raw `err_msg`.
    /// Surfaces like `runtime/mission.rs::process_mission_outcome_and_notify`
    /// render `format!("Mission failed: {error}")` directly, bypassing the
    /// channel-edge sanitizer in `bridge::user_facing_errors`, so any
    /// unclassified Monty output would leak tracebacks / internal paths
    /// there. The generic user-facing text now reads "internal orchestrator
    /// failure"; the raw message is preserved in `debug_detail`.
    #[test]
    fn failure_reason_hides_unknown_raw_message_from_user_text() {
        let failure =
            classify_orchestrator_failure("Orchestrator runtime error", "NameError: foo undefined");
        assert!(matches!(
            failure.kind,
            OrchestratorFailureKind::Other { .. }
        ));
        let rendered = failure.user_message();
        assert!(
            !rendered.contains("NameError"),
            "Other variant must not surface raw err_msg in Display, got: {rendered}"
        );
        assert!(
            rendered.contains("internal orchestrator failure"),
            "Other variant should render the generic fallback, got: {rendered}"
        );
        // Raw detail is still available for operator triage via debug_detail.
        assert!(
            failure.debug_detail().contains("NameError"),
            "debug_detail must preserve the raw err_msg, got: {}",
            failure.debug_detail()
        );
    }

    /// Regression for Copilot review on PR #2753 — substring `"duration"`
    /// alone mis-classified any error whose message happened to contain
    /// that word as a timeout. The narrow predicate set now requires
    /// the full phrase `"duration limit"` / `"max_duration"` /
    /// `"maximum duration"` or an explicit timeout word.
    #[test]
    fn failure_reason_does_not_treat_bare_duration_as_timeout() {
        let failure = classify_orchestrator_failure(
            "Orchestrator runtime error",
            "TypeError: duration must be a positive integer",
        );
        assert!(
            !matches!(failure.kind, OrchestratorFailureKind::TimeLimit { .. }),
            "bare 'duration' in an unrelated error must not classify as TimeLimit, got: {:?}",
            failure.kind
        );
        assert!(
            matches!(failure.kind, OrchestratorFailureKind::Other { .. }),
            "expected Other variant for unrelated duration-word error, got: {:?}",
            failure.kind
        );
    }

    #[test]
    fn failure_reason_strips_python_traceback() {
        // Regression for #2546 — bug bash 4/16 reported raw Python tracebacks
        // from the Monty VM being shown verbatim to end users, including
        // internal file paths ("orchestrator.py", line 907) and upstream
        // HTTP response bodies.
        let raw = "Traceback (most recent call last):\n  File \"orchestrator.py\", line 907, in run_loop\n  File \"orchestrator.py\", line 548, in __llm_complete__\nRuntimeError: LLM call failed: Provider nearai_chat request failed: HTTP 502 Bad Gateway";
        let failure = classify_orchestrator_failure("Orchestrator error after resume", raw);
        assert!(matches!(
            failure.kind,
            OrchestratorFailureKind::Traceback { .. }
        ));
        let rendered = failure.user_message();
        assert!(
            rendered.contains("internal orchestrator failure"),
            "should surface a generic internal-failure message, got: {rendered}"
        );
        for forbidden in [
            "Traceback",
            "orchestrator.py",
            "File \"",
            "line 907",
            "line 548",
            "HTTP 502",
        ] {
            assert!(
                !rendered.contains(forbidden),
                "user-visible reason must not leak `{forbidden}`, got: {rendered}"
            );
        }
        // The debug detail MUST retain the raw traceback so gateway
        // debug mode can surface it without re-reading logs.
        assert!(
            failure.debug_detail().contains("Traceback"),
            "debug detail must preserve the raw Monty trace, got: {}",
            failure.debug_detail()
        );
    }

    #[test]
    fn max_duration_default_and_bounds() {
        // Default (no env var set): 300s — but OnceLock may already be
        // primed by another test in the suite, so we only check it's within
        // the documented bounds.
        let secs = orchestrator_max_duration().as_secs();
        assert!(
            (ORCHESTRATOR_MIN_MAX_DURATION_SECS..=ORCHESTRATOR_MAX_MAX_DURATION_SECS)
                .contains(&secs),
            "orchestrator_max_duration must be within [{ORCHESTRATOR_MIN_MAX_DURATION_SECS}, {ORCHESTRATOR_MAX_MAX_DURATION_SECS}], got {secs}"
        );
    }

    // ── Python helper unit tests via Monty ──────────────────────
    //
    // Extracts the helper functions from the default orchestrator and
    // evaluates `signals_tool_intent(text)` directly, mirroring the V1
    // Rust unit test suite in crates/ironclaw_llm/src/reasoning.rs.

    /// Run a Python expression that returns a bool by prepending the
    /// orchestrator helper definitions and wrapping in `FINAL(expr)`.
    /// Run a Python snippet and drive the Monty VM, returning the FINAL()
    /// value as a `MontyObject`. This is the common core for `eval_python_bool`
    /// and `eval_python_int`.
    fn run_python_final(code: String) -> MontyObject {
        let runner =
            MontyRun::new(code, "test.py", vec![]).expect("Failed to parse orchestrator helpers");
        let mut stdout = String::new();
        let tracker = LimitedTracker::new(ResourceLimits::new().max_allocations(500_000));

        let mut progress = runner
            .start(vec![], tracker, PrintWriter::CollectString(&mut stdout))
            .expect("Failed to start orchestrator test");

        loop {
            match progress {
                RunProgress::Complete(obj) => return obj,
                RunProgress::FunctionCall(call) => {
                    if call.function_name == "FINAL" {
                        let val = call.args.first().cloned().unwrap_or(MontyObject::None);
                        let _ = call.resume(
                            ExtFunctionResult::Return(MontyObject::None),
                            PrintWriter::CollectString(&mut stdout),
                        );
                        return val;
                    }
                    let ext_result = match call.function_name.as_str() {
                        "__regex_match__" => handle_regex_match(&call.args),
                        _ => ExtFunctionResult::Return(MontyObject::None),
                    };
                    progress = call
                        .resume(ext_result, PrintWriter::CollectString(&mut stdout))
                        .expect("resume failed");
                }
                RunProgress::NameLookup(lookup) => {
                    progress = lookup
                        .resume(
                            NameLookupResult::Undefined,
                            PrintWriter::CollectString(&mut stdout),
                        )
                        .expect("name lookup resume failed");
                }
                _ => panic!("Unexpected RunProgress variant in test"),
            }
        }
    }

    fn eval_python_bool(expr: &str) -> bool {
        // Extract only the helper functions (everything before run_loop)
        let helpers_end = DEFAULT_ORCHESTRATOR
            .find("\ndef run_loop(")
            .unwrap_or(DEFAULT_ORCHESTRATOR.len());
        let helpers = &DEFAULT_ORCHESTRATOR[..helpers_end]; // safety: find() returns a char boundary on this ASCII-only constant

        let code = format!("{helpers}\nFINAL({expr})");
        match run_python_final(code) {
            MontyObject::Bool(v) => v,
            other => panic!("Expected bool, got: {other:?}"),
        }
    }

    /// Run a Python program (with orchestrator helpers in scope) that ends
    /// with `FINAL(int_expr)` and return the integer value.
    fn eval_python_int(program: &str) -> i64 {
        let helpers_end = DEFAULT_ORCHESTRATOR
            .find("\ndef run_loop(")
            .unwrap_or(DEFAULT_ORCHESTRATOR.len());
        let helpers = &DEFAULT_ORCHESTRATOR[..helpers_end];

        let code = format!("{helpers}\n{program}");
        match run_python_final(code) {
            MontyObject::Int(v) => v,
            other => panic!("Expected int, got: {other:?}"),
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

    // ── Stop / pause / cancel intent (mission lifecycle) ─────────

    #[test]
    fn signals_tool_intent_stop_pause_cancel() {
        assert!(eval_python_bool(
            r#"signals_tool_intent("I'll stop the mission now.")"#
        ));
        assert!(eval_python_bool(
            r#"signals_tool_intent("Let me pause the ticker.")"#
        ));
        assert!(eval_python_bool(
            r#"signals_tool_intent("I'll cancel the monitoring.")"#
        ));
        assert!(eval_python_bool(
            r#"signals_tool_intent("I'm going to halt the recurring task.")"#
        ));
        assert!(eval_python_bool(
            r#"signals_tool_intent("I will disable the mission.")"#
        ));
    }

    #[test]
    fn signals_tool_intent_no_false_positive_stop_discussion() {
        // "let me explain" is in EXCLUSIONS — blocks the entire text
        assert!(!eval_python_bool(
            r#"signals_tool_intent("Let me explain how to stop the mission.")"#
        ));
        // Past tense should not trigger
        assert!(!eval_python_bool(
            r#"signals_tool_intent("I already stopped the mission.")"#
        ));
    }

    // ── Execution intent: stop / pause / cancel ────────────────

    #[test]
    fn signals_execution_intent_stop_pause_cancel() {
        assert!(eval_python_bool(r#"signals_execution_intent("stop it")"#));
        assert!(eval_python_bool(
            r#"signals_execution_intent("pause the mission")"#
        ));
        assert!(eval_python_bool(
            r#"signals_execution_intent("cancel that")"#
        ));
        assert!(eval_python_bool(
            r#"signals_execution_intent("please stop the ticker")"#
        ));
        assert!(eval_python_bool(
            r#"signals_execution_intent("please pause everything")"#
        ));
    }

    #[test]
    fn signals_execution_intent_bare_stop() {
        // Bare imperative commands — the exact user messages from #2808
        assert!(eval_python_bool(r#"signals_execution_intent("stop")"#));
        assert!(eval_python_bool(
            r#"signals_execution_intent("stop pinging")"#
        ));
        assert!(eval_python_bool(r#"signals_execution_intent("pause")"#));
        assert!(eval_python_bool(r#"signals_execution_intent("cancel")"#));
        assert!(eval_python_bool(r#"signals_execution_intent("halt")"#));
    }

    #[test]
    fn signals_execution_intent_no_false_positive_stop_in_sentence() {
        // "stop" mid-sentence should NOT trigger — only at the start
        assert!(!eval_python_bool(
            r#"signals_execution_intent("I can't stop thinking about it")"#
        ));
        assert!(!eval_python_bool(
            r#"signals_execution_intent("how do I stop a mission?")"#
        ));
    }

    #[test]
    fn signals_execution_intent_halt_disable_phrases() {
        // "halt/disable" pronoun+article phrases and "please halt/disable"
        assert!(eval_python_bool(r#"signals_execution_intent("halt that")"#));
        assert!(eval_python_bool(
            r#"signals_execution_intent("halt the mission")"#
        ));
        assert!(eval_python_bool(
            r#"signals_execution_intent("disable it")"#
        ));
        assert!(eval_python_bool(
            r#"signals_execution_intent("disable the ticker")"#
        ));
        assert!(eval_python_bool(
            r#"signals_execution_intent("please halt the mission")"#
        ));
        assert!(eval_python_bool(
            r#"signals_execution_intent("please disable the routine")"#
        ));
        // Bare "disable" command
        assert!(eval_python_bool(r#"signals_execution_intent("disable")"#));
    }

    #[test]
    fn signals_execution_intent_bare_stop_with_punctuation() {
        // Bare commands with trailing punctuation must still match
        assert!(eval_python_bool(r#"signals_execution_intent("stop.")"#));
        assert!(eval_python_bool(r#"signals_execution_intent("cancel!")"#));
        assert!(eval_python_bool(
            r#"signals_execution_intent("stop pinging.")"#
        ));
    }

    // ── Skill activation: smart-quote / autocorrect resilience ───
    //
    // Regression for the ceo-setup non-activation report. iOS / macOS / most
    // rich text inputs autocorrect `I'm` (ASCII U+0027) to `I'm` (curly
    // U+2019). Authored regex patterns use ASCII punctuation, so without
    // boundary normalization the curly form silently fails to match and
    // the skill scores 0. `normalize_punctuation` in `default.py` folds
    // curly quotes/dashes to ASCII before scoring.

    #[test]
    fn normalize_punctuation_folds_curly_quotes_and_dashes() {
        // The input contains every typographic variant we fold. The
        // expected output uses only ASCII punctuation, so a Rust-side
        // regex that authors typed naturally still hits.
        let raw =
            "\u{2018}\u{2019}\u{201A}\u{201B}\u{201C}\u{201D}\u{201E}\u{201F}\u{2013}\u{2014}";
        let expected = "''''\"\"\"\"--";
        let program = format!("FINAL(normalize_punctuation({raw:?}) == {expected:?})");
        // Run the helper-only slice of default.py with the assertion appended.
        let helpers_end = DEFAULT_ORCHESTRATOR
            .find("\ndef run_loop(")
            .unwrap_or(DEFAULT_ORCHESTRATOR.len());
        let helpers = &DEFAULT_ORCHESTRATOR[..helpers_end];
        let code = format!("{helpers}\n{program}");
        match run_python_final(code) {
            MontyObject::Bool(true) => {}
            other => panic!("normalize_punctuation did not produce ASCII fold: {other:?}"),
        }
    }

    #[test]
    fn select_skills_matches_curly_apostrophe_input() {
        // The ceo-setup skill's first pattern uses ASCII `'`. A user
        // typing on iOS sends U+2019. With normalization, select_skills
        // must still pick the skill; without it, the regex misses and
        // the skill scores 0.
        let pattern = r"(?i)I'm a (CEO|manager|executive|director|VP|founder)";
        // Build a single-skill list as a Python literal — metadata shape
        // matches what handle_list_skills emits at runtime.
        let skill_literal = format!(
            r#"[{{"doc_id": "test", "title": "ceo-setup", "content": "body", "metadata": {{"name": "ceo-setup", "activation": {{"patterns": [{pattern:?}], "max_context_tokens": 2500, "keywords": [], "tags": []}}}}}}]"#
        );
        let curly_goal = "I\u{2019}m Illia Polosukhin. I\u{2019}m a CEO of NEAR Foundation";
        let program = format!(
            "selected = select_skills({skill_literal}, {curly_goal:?}); FINAL(len(selected))"
        );
        let helpers_end = DEFAULT_ORCHESTRATOR
            .find("\ndef run_loop(")
            .unwrap_or(DEFAULT_ORCHESTRATOR.len());
        let helpers = &DEFAULT_ORCHESTRATOR[..helpers_end];
        let code = format!("{helpers}\n{program}");
        match run_python_final(code) {
            MontyObject::Int(1) => {}
            other => {
                panic!("select_skills should pick ceo-setup for curly-quoted input, got: {other:?}")
            }
        }
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
            paused_lease: None,
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
        assert!(matches!(outcome, ThreadOutcome::Failed { error, .. } if error == "boom"));
    }

    #[test]
    fn parse_outcome_gate_paused() {
        let lease = crate::types::capability::CapabilityLease {
            id: crate::types::capability::LeaseId::new(),
            thread_id: crate::types::thread::ThreadId::new(),
            capability_name: "test-capability".into(),
            granted_actions: crate::types::capability::GrantedActions::Specific(vec![
                "shell".into(),
            ]),
            granted_at: chrono::Utc::now(),
            expires_at: None,
            max_uses: Some(1),
            uses_remaining: Some(1),
            revoked: false,
            revoked_reason: None,
        };
        let result = serde_json::json!({
            "outcome": "gate_paused",
            "gate_name": "approval",
            "action_name": "shell",
            "call_id": "abc",
            "parameters": {"cmd": "rm -rf /"},
            "resume_kind": {"Approval": {"allow_always": true}},
            "paused_lease": lease,
        });
        let outcome = parse_outcome(&result);
        assert!(matches!(
            outcome,
            ThreadOutcome::GatePaused {
                action_name,
                paused_lease: Some(_),
                ..
            } if action_name == "shell"
        ));
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

    /// Regression test for nearai/ironclaw#2084 — drives the private
    /// `handle_list_skills` call site end-to-end (not just the
    /// `list_skills_global` helper). This is the caller-level test required by
    /// `.claude/rules/testing.md` ("Test Through the Caller, Not Just the
    /// Helper"): a future regression that reverts `handle_list_skills` back to
    /// `list_memory_docs_with_shared(thread.project_id, &thread.user_id)` would
    /// slip past a helper-only unit test but must fail this one, because the
    /// shared skill lives in a different project than the caller's thread.
    #[tokio::test]
    async fn handle_list_skills_returns_shared_skills_from_other_projects() {
        use crate::types::shared_owner_id;
        use crate::types::thread::{ThreadConfig, ThreadType};

        // project_a: where alice's thread runs.
        // project_b: where the admin installed a shared skill.
        let project_a = ProjectId::new();
        let project_b = ProjectId::new();

        let shared_skill = MemoryDoc::new(
            project_b,
            shared_owner_id(),
            DocType::Skill,
            "skill:admin-installed",
            "shared content",
        );
        let alice_skill = MemoryDoc::new(
            project_a,
            "alice",
            DocType::Skill,
            "skill:alice-owned",
            "alice content",
        );
        // A non-skill doc in alice's project must not leak into the result.
        let alice_note = MemoryDoc::new(
            project_a,
            "alice",
            DocType::Note,
            "note:scratch",
            "note body",
        );

        let store: Arc<dyn Store> = Arc::new(crate::tests::InMemoryStore::with_docs(vec![
            shared_skill.clone(),
            alice_skill.clone(),
            alice_note,
        ]));

        let thread = Thread::new(
            "test goal",
            ThreadType::Foreground,
            project_a,
            "alice",
            ThreadConfig::default(),
        );

        let result = handle_list_skills(&[], &thread, Some(&store)).await;
        let ExtFunctionResult::Return(obj) = result else {
            panic!("handle_list_skills did not return a value");
        };
        let json = monty_to_json(&obj);
        let arr = json
            .as_array()
            .expect("handle_list_skills must return a JSON array");

        let titles: Vec<&str> = arr
            .iter()
            .filter_map(|v| v.get("title").and_then(|t| t.as_str()))
            .collect();

        assert!(
            titles.contains(&"skill:admin-installed"),
            "shared skill from project_b must be visible to alice's thread in project_a — got {titles:?}"
        );
        assert!(
            titles.contains(&"skill:alice-owned"),
            "alice's own skill must be visible — got {titles:?}"
        );
        assert!(
            !titles.contains(&"note:scratch"),
            "non-skill docs must be filtered out — got {titles:?}"
        );
        assert_eq!(
            arr.len(),
            2,
            "expected exactly 2 skills (shared + alice), got {}: {titles:?}",
            arr.len()
        );
    }

    // ── handle_llm_complete model forwarding ────────────────────

    /// LLM backend that records the model from each `complete()` call.
    /// Used to verify the orchestrator's __llm_complete__ host fn forwards
    /// `explicit_config["model"]` onto `LlmCallConfig.model`.
    struct ModelCapturingLlm {
        captured: tokio::sync::Mutex<Vec<Option<String>>>,
    }

    #[async_trait::async_trait]
    impl LlmBackend for ModelCapturingLlm {
        fn model_name(&self) -> &str {
            "capturing"
        }

        async fn complete(
            &self,
            _messages: &[ThreadMessage],
            _actions: &[crate::types::capability::ActionDef],
            config: &LlmCallConfig,
        ) -> Result<crate::traits::llm::LlmOutput, EngineError> {
            self.captured.lock().await.push(config.model.clone());
            Ok(crate::traits::llm::LlmOutput {
                response: crate::types::step::LlmResponse::Text("ok".into()),
                usage: crate::types::step::TokenUsage::default(),
            })
        }
    }

    /// No-op effect executor — handle_llm_complete only consults it for
    /// `available_actions(...)`, which we satisfy with an empty list.
    struct NoopEffects;

    #[async_trait::async_trait]
    impl EffectExecutor for NoopEffects {
        async fn execute_action(
            &self,
            _: &str,
            _: serde_json::Value,
            _: &crate::types::capability::CapabilityLease,
            _: &ThreadExecutionContext,
        ) -> Result<crate::types::step::ActionResult, EngineError> {
            Ok(crate::types::step::ActionResult {
                call_id: String::new(),
                action_name: String::new(),
                output: serde_json::json!({}),
                is_error: false,
                duration: std::time::Duration::from_millis(1),
            })
        }

        async fn available_actions(
            &self,
            _: &[crate::types::capability::CapabilityLease],
            _: &ThreadExecutionContext,
        ) -> Result<Vec<crate::types::capability::ActionDef>, EngineError> {
            Ok(vec![])
        }

        async fn available_capabilities(
            &self,
            _: &[crate::types::capability::CapabilityLease],
            _: &ThreadExecutionContext,
        ) -> Result<Vec<crate::types::capability::CapabilitySummary>, EngineError> {
            Ok(vec![])
        }
    }

    struct PromptCapturingLlm {
        captured_messages: tokio::sync::Mutex<Vec<Vec<ThreadMessage>>>,
    }

    #[async_trait::async_trait]
    impl LlmBackend for PromptCapturingLlm {
        fn model_name(&self) -> &str {
            "prompt-capturing"
        }

        async fn complete(
            &self,
            messages: &[ThreadMessage],
            _actions: &[crate::types::capability::ActionDef],
            _config: &LlmCallConfig,
        ) -> Result<crate::traits::llm::LlmOutput, EngineError> {
            self.captured_messages.lock().await.push(messages.to_vec());
            Ok(crate::traits::llm::LlmOutput {
                response: crate::types::step::LlmResponse::Text("ok".into()),
                usage: crate::types::step::TokenUsage::default(),
            })
        }
    }

    struct CompactActionEffects;

    #[async_trait::async_trait]
    impl EffectExecutor for CompactActionEffects {
        async fn execute_action(
            &self,
            _: &str,
            _: serde_json::Value,
            _: &crate::types::capability::CapabilityLease,
            _: &ThreadExecutionContext,
        ) -> Result<crate::types::step::ActionResult, EngineError> {
            Ok(crate::types::step::ActionResult {
                call_id: String::new(),
                action_name: String::new(),
                output: serde_json::json!({}),
                is_error: false,
                duration: std::time::Duration::from_millis(1),
            })
        }

        async fn available_actions(
            &self,
            _: &[crate::types::capability::CapabilityLease],
            _: &ThreadExecutionContext,
        ) -> Result<Vec<crate::types::capability::ActionDef>, EngineError> {
            Ok(vec![crate::types::capability::ActionDef {
                name: "gmail_send".into(),
                description: "Send Gmail".into(),
                parameters_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "to": {"type": "string"},
                        "body": {"type": "string"}
                    }
                }),
                effects: vec![crate::types::capability::EffectType::WriteExternal],
                requires_approval: false,
                model_tool_surface: crate::types::capability::ModelToolSurface::CompactToolInfo,
                discovery: None,
            }])
        }

        async fn available_capabilities(
            &self,
            _: &[crate::types::capability::CapabilityLease],
            _: &ThreadExecutionContext,
        ) -> Result<Vec<crate::types::capability::CapabilitySummary>, EngineError> {
            Ok(vec![])
        }
    }

    struct InventoryErrorEffects;

    #[async_trait::async_trait]
    impl EffectExecutor for InventoryErrorEffects {
        async fn execute_action(
            &self,
            action_name: &str,
            _: serde_json::Value,
            _: &crate::types::capability::CapabilityLease,
            ctx: &ThreadExecutionContext,
        ) -> Result<crate::types::step::ActionResult, EngineError> {
            Ok(crate::types::step::ActionResult {
                call_id: String::new(),
                action_name: action_name.to_string(),
                output: serde_json::json!({
                    "has_action_snapshot": ctx.available_actions_snapshot.is_some(),
                    "has_inventory_snapshot": ctx.available_action_inventory_snapshot.is_some(),
                }),
                is_error: false,
                duration: std::time::Duration::from_millis(1),
            })
        }

        async fn available_actions(
            &self,
            _: &[crate::types::capability::CapabilityLease],
            _: &ThreadExecutionContext,
        ) -> Result<Vec<crate::types::capability::ActionDef>, EngineError> {
            Ok(vec![])
        }

        async fn available_action_inventory(
            &self,
            _: &[crate::types::capability::CapabilityLease],
            _: &ThreadExecutionContext,
        ) -> Result<crate::types::capability::ActionInventory, EngineError> {
            Err(EngineError::Effect {
                reason: "inventory failed".into(),
            })
        }

        async fn available_capabilities(
            &self,
            _: &[crate::types::capability::CapabilityLease],
            _: &ThreadExecutionContext,
        ) -> Result<Vec<crate::types::capability::CapabilitySummary>, EngineError> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn llm_complete_forwards_model_from_explicit_config() {
        let concrete = Arc::new(ModelCapturingLlm {
            captured: tokio::sync::Mutex::new(Vec::new()),
        });
        let llm: Arc<dyn LlmBackend> = Arc::clone(&concrete) as Arc<dyn LlmBackend>;
        let effects: Arc<dyn EffectExecutor> = Arc::new(NoopEffects);
        let leases = Arc::new(LeaseManager::new());
        let store: Arc<dyn Store> = Arc::new(crate::tests::InMemoryStore::with_docs(vec![]));

        let mut thread = Thread::new(
            "goal",
            crate::types::thread::ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            crate::types::thread::ThreadConfig::default(),
        );
        thread.transition_to(ThreadState::Running, None).unwrap();

        // Build the args __llm_complete__ receives from Python:
        // (messages, actions, config). config = {"model": "gpt-4o"}.
        let mut total_tokens = TokenUsage::default();
        let result = handle_llm_complete(
            &[
                json_to_monty(&serde_json::json!([{"role":"user","content":"hi"}])),
                json_to_monty(&serde_json::json!([])),
                json_to_monty(&serde_json::json!({"model": "gpt-4o"})),
            ],
            &[],
            &mut thread,
            LlmCompleteDeps {
                llm: &llm,
                effects: &effects,
                leases: &leases,
                store: Some(&store),
                platform_info: None,
            },
            &mut total_tokens,
        )
        .await;

        assert!(matches!(result, ExtFunctionResult::Return(_)));
        let captured = concrete.captured.lock().await;
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].as_deref(), Some("gpt-4o"));
    }

    #[tokio::test]
    async fn llm_complete_without_model_passes_none() {
        let concrete = Arc::new(ModelCapturingLlm {
            captured: tokio::sync::Mutex::new(Vec::new()),
        });
        let llm: Arc<dyn LlmBackend> = Arc::clone(&concrete) as Arc<dyn LlmBackend>;
        let effects: Arc<dyn EffectExecutor> = Arc::new(NoopEffects);
        let leases = Arc::new(LeaseManager::new());
        let store: Arc<dyn Store> = Arc::new(crate::tests::InMemoryStore::with_docs(vec![]));

        let mut thread = Thread::new(
            "goal",
            crate::types::thread::ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            crate::types::thread::ThreadConfig::default(),
        );
        thread.transition_to(ThreadState::Running, None).unwrap();

        let mut total_tokens = TokenUsage::default();
        let _ = handle_llm_complete(
            &[
                json_to_monty(&serde_json::json!([{"role":"user","content":"hi"}])),
                json_to_monty(&serde_json::json!([])),
                json_to_monty(&serde_json::json!({"max_tokens": 100})),
            ],
            &[],
            &mut thread,
            LlmCompleteDeps {
                llm: &llm,
                effects: &effects,
                leases: &leases,
                store: Some(&store),
                platform_info: None,
            },
            &mut total_tokens,
        )
        .await;

        let captured = concrete.captured.lock().await;
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0], None);
    }

    #[tokio::test]
    async fn llm_complete_refreshes_codeact_prompt_with_current_compact_actions() {
        let concrete = Arc::new(PromptCapturingLlm {
            captured_messages: tokio::sync::Mutex::new(Vec::new()),
        });
        let llm: Arc<dyn LlmBackend> = Arc::clone(&concrete) as Arc<dyn LlmBackend>;
        let effects: Arc<dyn EffectExecutor> = Arc::new(CompactActionEffects);
        let leases = Arc::new(LeaseManager::new());
        let store: Arc<dyn Store> = Arc::new(crate::tests::InMemoryStore::with_docs(vec![]));

        let mut thread = Thread::new(
            "goal",
            crate::types::thread::ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            crate::types::thread::ThreadConfig::default(),
        );
        thread.transition_to(ThreadState::Running, None).unwrap();
        thread.messages = vec![
            ThreadMessage::system(
                crate::executor::prompt::build_codeact_system_prompt_with_docs(&[], &[], &[], None),
            ),
            ThreadMessage::user("use gmail"),
        ];
        leases
            .grant(
                thread.id,
                "tools",
                crate::types::capability::GrantedActions::All,
                None,
                None,
            )
            .await
            .expect("grant tool lease");

        let mut total_tokens = TokenUsage::default();
        let result = handle_llm_complete(
            &[],
            &[],
            &mut thread,
            LlmCompleteDeps {
                llm: &llm,
                effects: &effects,
                leases: &leases,
                store: Some(&store),
                platform_info: None,
            },
            &mut total_tokens,
        )
        .await;

        assert!(matches!(result, ExtFunctionResult::Return(_)));
        let captured = concrete.captured_messages.lock().await;
        let system_prompt = &captured[0][0].content;
        assert!(system_prompt.contains("## Enabled Tools"));
        assert!(system_prompt.contains("`gmail_send`"));
    }

    #[tokio::test]
    async fn execute_action_does_not_set_empty_snapshots_when_inventory_fetch_fails() {
        let effects: Arc<dyn EffectExecutor> = Arc::new(InventoryErrorEffects);
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());

        let mut thread = Thread::new(
            "test action inventory fallback",
            crate::types::thread::ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            crate::types::thread::ThreadConfig::default(),
        );
        thread.transition_to(ThreadState::Running, None).unwrap();

        leases
            .grant(
                thread.id,
                "tools",
                crate::types::capability::GrantedActions::All,
                None,
                None,
            )
            .await
            .expect("grant lease");

        let controller = crate::gate::CancellingGateController::arc();
        let result = handle_execute_action(
            &[
                MontyObject::String("echo".into()),
                json_to_monty(&serde_json::json!({})),
            ],
            &[(
                MontyObject::String("call_id".into()),
                MontyObject::String("call-1".into()),
            )],
            &mut thread,
            &effects,
            &leases,
            &policy,
            None,
            &controller,
        )
        .await;

        let ExtFunctionResult::Return(obj) = result else {
            panic!("handle_execute_action did not return a value");
        };
        let json = monty_to_json(&obj);
        assert_eq!(json["is_error"], serde_json::json!(false));
        assert_eq!(
            json["output"],
            serde_json::json!({
                "has_action_snapshot": false,
                "has_inventory_snapshot": false,
            })
        );
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

    // ── Consecutive action error counting (issue #2325) ──────────
    //
    // The run_loop tracks `consecutive_action_errors` for Tier 0 (structured
    // action calls). These tests exercise the counting logic extracted from
    // run_loop into small Python snippets that simulate batch outcomes.

    #[test]
    fn action_errors_increment_when_all_actions_fail() {
        // Simulate 3 consecutive batches where all actions fail.
        let count = eval_python_int(
            r#"
consecutive_action_errors = 0
for _ in range(3):
    batch_error_count = 2
    batch_success_count = 0
    if batch_success_count > 0:
        consecutive_action_errors = 0
    elif batch_error_count > 0:
        consecutive_action_errors += 1
FINAL(consecutive_action_errors)
"#,
        );
        assert_eq!(count, 3);
    }

    #[test]
    fn action_errors_reset_when_any_action_succeeds() {
        // 2 all-fail batches, then 1 batch with a success => resets to 0.
        let count = eval_python_int(
            r#"
consecutive_action_errors = 0
for batch in [(0, 2), (0, 1), (1, 1)]:
    batch_success_count = batch[0]
    batch_error_count = batch[1]
    if batch_success_count > 0:
        consecutive_action_errors = 0
    elif batch_error_count > 0:
        consecutive_action_errors += 1
FINAL(consecutive_action_errors)
"#,
        );
        assert_eq!(count, 0);
    }

    #[test]
    fn action_errors_partial_success_resets_counter() {
        // A batch with mixed results (some succeed, some fail) should reset.
        let count = eval_python_int(
            r#"
consecutive_action_errors = 5
batch_success_count = 1
batch_error_count = 3
if batch_success_count > 0:
    consecutive_action_errors = 0
elif batch_error_count > 0:
    consecutive_action_errors += 1
FINAL(consecutive_action_errors)
"#,
        );
        assert_eq!(count, 0);
    }

    /// Regression: `max_consecutive_errors` arrives as `null` when the Rust
    /// caller passes `Option::None`. Python's `dict.get(key, default)` returns
    /// the explicit `None`, not the default, so `None + 2` used to blow up in
    /// the error-gating branch on the very first failed action call. The
    /// orchestrator now coalesces `None` to a sentinel; this test pins that
    /// behavior.
    #[test]
    fn action_errors_tolerate_null_max_consecutive_errors() {
        let result = eval_python_int(
            r#"
max_consecutive_errors = None
if max_consecutive_errors is None:
    max_consecutive_errors = 10**9
consecutive_action_errors = 1
nudge = False
failed = False
if consecutive_action_errors > 0 and consecutive_action_errors >= max_consecutive_errors + 2:
    failed = True
elif consecutive_action_errors > 0 and consecutive_action_errors >= max_consecutive_errors:
    nudge = True
# 0 = no nudge / no failure (expected with None = no-limit sentinel)
if failed:
    FINAL(2)
elif nudge:
    FINAL(1)
else:
    FINAL(0)
"#,
        );
        assert_eq!(
            result, 0,
            "None max_consecutive_errors must behave as no-limit, not crash"
        );
    }

    #[test]
    fn action_errors_nudge_injected_at_threshold() {
        // When consecutive_action_errors reaches max_consecutive_errors,
        // a nudge message should be appended. We simulate the branching
        // logic and check whether a nudge would fire.
        // Returns 1 if nudge fires (not failure), 0 otherwise.
        let result = eval_python_int(
            r#"
max_consecutive_errors = 5
consecutive_action_errors = 5
nudge = False
failed = False
if consecutive_action_errors > 0 and consecutive_action_errors >= max_consecutive_errors + 2:
    failed = True
elif consecutive_action_errors > 0 and consecutive_action_errors >= max_consecutive_errors:
    nudge = True
if nudge and not failed:
    FINAL(1)
else:
    FINAL(0)
"#,
        );
        assert_eq!(result, 1, "nudge should fire at threshold");
    }

    #[test]
    fn action_errors_no_nudge_below_threshold() {
        // Returns 1 if nudge fires, 0 if not.
        let result = eval_python_int(
            r#"
max_consecutive_errors = 5
consecutive_action_errors = 4
nudge = False
failed = False
if consecutive_action_errors > 0 and consecutive_action_errors >= max_consecutive_errors + 2:
    failed = True
elif consecutive_action_errors > 0 and consecutive_action_errors >= max_consecutive_errors:
    nudge = True
if nudge:
    FINAL(1)
else:
    FINAL(0)
"#,
        );
        assert_eq!(result, 0, "nudge should not fire below threshold");
    }

    #[test]
    fn action_errors_failure_at_threshold_plus_two() {
        // At max_consecutive_errors + 2, the thread should transition to failed.
        // Returns 1 if failed, 0 if not.
        let result = eval_python_int(
            r#"
max_consecutive_errors = 5
consecutive_action_errors = 7
failed = False
if consecutive_action_errors > 0 and consecutive_action_errors >= max_consecutive_errors + 2:
    failed = True
if failed:
    FINAL(1)
else:
    FINAL(0)
"#,
        );
        assert_eq!(result, 1, "should fail at threshold + 2");
    }

    #[test]
    fn action_errors_nudge_at_threshold_not_failure() {
        // At exactly max_consecutive_errors + 1, we get a nudge but not failure.
        let result = eval_python_int(
            r#"
max_consecutive_errors = 5
consecutive_action_errors = 6
nudge = False
failed = False
if consecutive_action_errors > 0 and consecutive_action_errors >= max_consecutive_errors + 2:
    failed = True
elif consecutive_action_errors > 0 and consecutive_action_errors >= max_consecutive_errors:
    nudge = True
# Return 0=nothing, 1=nudge, 2=failed
if failed:
    FINAL(2)
elif nudge:
    FINAL(1)
else:
    FINAL(0)
"#,
        );
        assert_eq!(result, 1, "should nudge at threshold + 1, not fail");
    }

    #[test]
    fn action_errors_none_limit_skips_check_without_typeerror() {
        // Regression: when max_consecutive_errors is None (meaning "no limit"),
        // the arithmetic `max_consecutive_errors + 2` used to crash with
        // TypeError on the first action error. The guard must short-circuit
        // on None and leave both the nudge and failure branches untaken.
        let result = eval_python_int(
            r#"
max_consecutive_errors = None
consecutive_action_errors = 1
nudge = False
failed = False
if max_consecutive_errors is not None and consecutive_action_errors > 0 and consecutive_action_errors >= max_consecutive_errors + 2:
    failed = True
elif max_consecutive_errors is not None and consecutive_action_errors > 0 and consecutive_action_errors >= max_consecutive_errors:
    nudge = True
# Return 0=nothing, 1=nudge, 2=failed
if failed:
    FINAL(2)
elif nudge:
    FINAL(1)
else:
    FINAL(0)
"#,
        );
        assert_eq!(result, 0, "None limit should disable the guard entirely");
    }

    #[test]
    fn code_errors_none_limit_skips_failure_check() {
        // Regression: same None-guard for the code-error branch at line 660.
        let result = eval_python_int(
            r#"
max_consecutive_errors = None
consecutive_errors = 99
failed = False
if max_consecutive_errors is not None and consecutive_errors >= max_consecutive_errors:
    failed = True
if failed:
    FINAL(1)
else:
    FINAL(0)
"#,
        );
        assert_eq!(
            result, 0,
            "None limit should not trigger failure regardless of consecutive_errors"
        );
    }

    #[test]
    fn action_error_prefix_added_to_error_output() {
        // Verify that [ACTION FAILED] prefix is prepended to error outputs.
        // Returns 1 if prefix present, 0 if not.
        let result = eval_python_int(
            r#"
r = {"action_name": "http", "output": "connection refused", "is_error": True}
output = r.get("output")
output_str = str(output) if output is not None else "[no output]"
if r.get("is_error"):
    output_str = "[ACTION FAILED] " + output_str
if output_str.startswith("[ACTION FAILED]"):
    FINAL(1)
else:
    FINAL(0)
"#,
        );
        assert_eq!(result, 1, "error outputs must get [ACTION FAILED] prefix");
    }

    #[test]
    fn action_error_skipped_calls_count_as_errors() {
        // When a call has no result (r is None), it should count as an error.
        let count = eval_python_int(
            r#"
batch_error_count = 0
batch_success_count = 0
r = None
if r is not None:
    if r.get("is_error"):
        batch_error_count += 1
    else:
        batch_success_count += 1
else:
    batch_error_count += 1
FINAL(batch_error_count)
"#,
        );
        assert_eq!(count, 1, "skipped calls must count as batch errors");
    }

    #[test]
    fn checkpoint_includes_consecutive_action_errors() {
        // Test that handle_save_checkpoint persists consecutive_action_errors
        // in the thread metadata.
        let mut thread = Thread::new(
            "goal",
            crate::types::thread::ThreadType::Foreground,
            ProjectId::new(),
            "user",
            crate::types::thread::ThreadConfig::default(),
        );
        thread.transition_to(ThreadState::Running, None).unwrap();

        let state = json_to_monty(&serde_json::json!({}));
        let counters = json_to_monty(&serde_json::json!({
            "nudge_count": 0,
            "consecutive_errors": 1,
            "consecutive_action_errors": 4,
            "compaction_count": 2,
        }));

        handle_save_checkpoint(&[state, counters], &[], &mut thread);

        let checkpoint = thread
            .metadata
            .get("runtime_checkpoint")
            .expect("checkpoint must exist");
        assert_eq!(
            checkpoint
                .get("consecutive_action_errors")
                .and_then(|v| v.as_u64()),
            Some(4),
            "consecutive_action_errors must be persisted in checkpoint"
        );
        assert_eq!(
            checkpoint
                .get("consecutive_errors")
                .and_then(|v| v.as_u64()),
            Some(1),
        );
        assert_eq!(
            checkpoint.get("compaction_count").and_then(|v| v.as_u64()),
            Some(2),
        );
    }

    /// Regression test: every assistant tool_call must have a matching
    /// ActionResult after parsing. If an ActionResult is missing, the LLM
    /// API rejects with "No tool output found for function call <id>".
    ///
    /// This was the root cause of the HTTP 400 from the OpenAI Codex
    /// provider: a tool returning null output caused the Python
    /// orchestrator to skip appending the ActionResult.
    #[test]
    fn json_to_thread_messages_every_tool_call_has_action_result() {
        // Simulate working_messages after the Python fix: every call gets
        // an ActionResult, even when the original output was null.
        let messages = serde_json::json!([
            {"role": "System", "content": "You are a helpful assistant."},
            {"role": "User", "content": "Update all tools."},
            {
                "role": "Assistant",
                "content": "",
                "action_calls": [
                    {"call_id": "call_AAA", "name": "tool_a", "params": {}},
                    {"call_id": "call_BBB", "name": "tool_b", "params": {}},
                    {"call_id": "call_CCC", "name": "tool_c", "params": {}}
                ]
            },
            {
                "role": "ActionResult",
                "content": "{\"ok\": true}",
                "action_name": "tool_a",
                "action_call_id": "call_AAA"
            },
            {
                "role": "ActionResult",
                "content": "[no output]",
                "action_name": "tool_b",
                "action_call_id": "call_BBB"
            },
            {
                "role": "ActionResult",
                "content": "{\"done\": true}",
                "action_name": "tool_c",
                "action_call_id": "call_CCC"
            }
        ]);

        let parsed = json_to_thread_messages(&messages).expect("must parse");
        assert_eq!(parsed.len(), 6);

        // Extract call IDs from the assistant message
        let assistant_calls: std::collections::HashSet<String> = parsed
            .iter()
            .filter_map(|m| m.action_calls.as_ref())
            .flat_map(|calls| calls.iter().map(|c| c.id.clone()))
            .collect();

        // Extract call IDs from ActionResult messages
        let result_call_ids: std::collections::HashSet<String> = parsed
            .iter()
            .filter(|m| m.role == crate::types::message::MessageRole::ActionResult)
            .filter_map(|m| m.action_call_id.clone())
            .collect();

        // Every tool_call must have a matching ActionResult
        for call_id in &assistant_calls {
            assert!(
                result_call_ids.contains(call_id),
                "tool_call {call_id} has no matching ActionResult — \
                 this would cause 'No tool output found' from the LLM API"
            );
        }
    }

    // ── CodeExecutionFailed event emission (caller test) ────────

    #[tokio::test]
    async fn execute_code_step_emits_code_execution_failed_event() {
        let llm: Arc<dyn LlmBackend> = Arc::new(ModelCapturingLlm {
            captured: tokio::sync::Mutex::new(Vec::new()),
        });
        let effects: Arc<dyn EffectExecutor> = Arc::new(NoopEffects);
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());

        let mut thread = Thread::new(
            "test code execution failure instrumentation",
            crate::types::thread::ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            crate::types::thread::ThreadConfig::default(),
        );
        thread.transition_to(ThreadState::Running, None).unwrap();

        // Pass intentionally broken Python code (syntax error)
        let args = &[
            json_to_monty(&serde_json::json!("def ==")),
            json_to_monty(&serde_json::json!({})),
        ];

        let (tx, _rx) = tokio::sync::broadcast::channel(16);
        let controller = crate::gate::CancellingGateController::arc();
        let _result = handle_execute_code_step(
            args,
            &[],
            &mut thread,
            &llm,
            &effects,
            &leases,
            &policy,
            Some(&tx),
            &controller,
        )
        .await;

        // Verify CodeExecutionFailed event was emitted on thread.events
        let code_failed_events: Vec<_> = thread
            .events
            .iter()
            .filter(|e| matches!(&e.kind, EventKind::CodeExecutionFailed { .. }))
            .collect();

        assert_eq!(
            code_failed_events.len(),
            1,
            "expected exactly one CodeExecutionFailed event, got {}",
            code_failed_events.len()
        );

        if let EventKind::CodeExecutionFailed {
            category,
            code_hash,
            ..
        } = &code_failed_events[0].kind
        {
            assert_eq!(
                *category,
                crate::types::step::CodeExecutionFailure::SyntaxError
            );
            assert!(code_hash.is_some());
        } else {
            panic!("expected CodeExecutionFailed event kind");
        }

        // Also verify ActionFailed was emitted (existing behavior)
        let action_failed = thread
            .events
            .iter()
            .any(|e| matches!(&e.kind, EventKind::ActionFailed { .. }));
        assert!(
            action_failed,
            "expected ActionFailed event alongside CodeExecutionFailed"
        );
    }
}
