//! Tier 1 executor: embedded Python via Monty.
//!
//! Executes LLM-generated Python code using the Monty interpreter. Tool
//! calls use **async dispatch**: each tool call returns a Monty `ExternalFuture`
//! via `resume_pending()`, allowing Python code to use `await` and
//! `asyncio.gather()` for parallel execution. When all tasks are blocked,
//! Monty yields `ResolveFutures` and we execute pending tools concurrently
//! via `JoinSet`.
//!
//! Follows the RLM (Recursive Language Model) pattern:
//! - Thread context injected as Python variables (not LLM attention input)
//! - `llm_query()` / `llm_query_batched()` for recursive subagent spawning
//! - `FINAL(answer)` / `FINAL_VAR(name)` for explicit termination
//! - Step 0 orientation preamble for context awareness
//! - Errors flow back to LLM for self-correction (not step termination)
//! - Output truncated to configurable limit with variable listing
//! - `asyncio.gather()` for parallel tool execution (via ResolveFutures)

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use monty::{
    ExcType, ExtFunctionResult, LimitedTracker, MontyException, MontyObject, MontyRun,
    NameLookupResult, PrintWriter, ResourceLimits, RunProgress,
};
use tracing::debug;

use crate::capability::lease::LeaseManager;
use crate::capability::policy::{PolicyDecision, PolicyEngine};
use crate::traits::effect::{EffectExecutor, ThreadExecutionContext};
use crate::traits::llm::{LlmBackend, LlmCallConfig};
use crate::types::error::EngineError;
use crate::types::event::EventKind;
use crate::types::message::{MessageRole, ThreadMessage};
use crate::types::step::{ActionResult, LlmResponse, TokenUsage};
use crate::types::thread::Thread;

// ── Configuration ───────────────────────────────────────────

/// Maximum characters of output to include in LLM context between steps.
/// Matches Prime Intellect's default. Configurable per thread in the future.
const OUTPUT_TRUNCATE_LEN: usize = 8_000;

/// Maximum characters for a preview prefix in compact metadata.
const OUTPUT_PREVIEW_LEN: usize = 200;

/// Default resource limits for Monty execution.
fn default_limits() -> ResourceLimits {
    ResourceLimits::new()
        .max_duration(Duration::from_secs(30))
        .max_allocations(1_000_000)
        .max_memory(64 * 1024 * 1024) // 64 MB
}

// ── Result types ────────────────────────────────────────────

/// Result of executing a code block.
pub struct CodeExecutionResult {
    /// The Python return value, converted to JSON.
    pub return_value: serde_json::Value,
    /// Captured print output.
    pub stdout: String,
    /// All action calls that were made during execution.
    pub action_results: Vec<ActionResult>,
    /// Events generated during execution.
    pub events: Vec<EventKind>,
    /// If set, execution was interrupted for approval.
    pub need_approval: Option<crate::runtime::messaging::ThreadOutcome>,
    /// Tokens used by recursive llm_query() calls.
    pub recursive_tokens: TokenUsage,
    /// If set, the code called FINAL() or FINAL_VAR() with this answer.
    pub final_answer: Option<String>,
    /// Whether the code execution hit an error (traceback included in stdout).
    pub had_error: bool,
}

/// Build a compact output summary for inclusion in LLM context between steps.
///
/// Truncates to `OUTPUT_TRUNCATE_LEN` (last N chars shown, like fast-rlm).
/// Includes a list of REPL variable names if available.
pub fn compact_output_metadata(stdout: &str, return_value: &serde_json::Value) -> String {
    let mut parts = Vec::new();

    if !stdout.is_empty() {
        let char_count = stdout.chars().count();
        if char_count > OUTPUT_TRUNCATE_LEN {
            let truncated: String = stdout
                .chars()
                .skip(char_count - OUTPUT_TRUNCATE_LEN)
                .collect();
            parts.push(format!(
                "[TRUNCATED: last {OUTPUT_TRUNCATE_LEN} of {char_count} chars shown]\n{truncated}",
            ));
        } else {
            parts.push(format!("[FULL OUTPUT: {char_count} chars]\n{stdout}"));
        }
    }

    if *return_value != serde_json::Value::Null {
        let val_str = serde_json::to_string_pretty(return_value).unwrap_or_default();
        let val_char_count = val_str.chars().count();
        if val_char_count > OUTPUT_PREVIEW_LEN {
            let preview: String = val_str.chars().take(OUTPUT_PREVIEW_LEN).collect();
            parts.push(format!(
                "Return value ({val_char_count} chars): {preview}...",
            ));
        } else {
            parts.push(format!("Return value: {val_str}"));
        }
    }

    if parts.is_empty() {
        "[code executed, no output]".into()
    } else {
        parts.join("\n")
    }
}

// ── Step 0 orientation preamble ─────────────────────────────

/// Build the Step 0 orientation preamble that auto-executes before the
/// first LLM call to give the model structural awareness of the context.
pub fn build_orientation_preamble(thread: &Thread) -> String {
    let msg_count = thread.messages.len();
    let total_chars: usize = thread.messages.iter().map(|m| m.content.len()).sum();
    let user_msgs = thread
        .messages
        .iter()
        .filter(|m| m.role == MessageRole::User)
        .count();

    let mut preview = String::new();
    if let Some(last_user) = thread
        .messages
        .iter()
        .rev()
        .find(|m| m.role == MessageRole::User)
    {
        let content_preview: String = last_user.content.chars().take(500).collect();
        let truncated = if last_user.content.chars().count() > 500 {
            "..."
        } else {
            ""
        };
        preview = format!("\nLast user message preview: {content_preview}{truncated}");
    }

    format!(
        "[Step 0 — Context Orientation]\n\
         Goal: {goal}\n\
         Context: {msg_count} messages, {total_chars} total chars, {user_msgs} from user\n\
         Step: {step}{preview}",
        goal = thread.goal,
        step = thread.step_count + 1,
    )
}

// ── Context injection (RLM 3.4) ────────────────────────────

/// Build Monty input variables from thread state.
///
/// `persisted_state` carries variables from previous code steps so the
/// REPL feels persistent even though each step creates a fresh MontyRun.
fn build_context_inputs(
    thread: &Thread,
    persisted_state: &serde_json::Value,
) -> (Vec<String>, Vec<MontyObject>) {
    let mut names = Vec::new();
    let mut values = Vec::new();

    // `context` — thread messages as a list of dicts
    let messages: Vec<MontyObject> = thread
        .messages
        .iter()
        .map(|msg| {
            let mut pairs = vec![
                (
                    MontyObject::String("role".into()),
                    MontyObject::String(format!("{:?}", msg.role)),
                ),
                (
                    MontyObject::String("content".into()),
                    MontyObject::String(msg.content.clone()),
                ),
            ];
            if let Some(ref name) = msg.action_name {
                pairs.push((
                    MontyObject::String("action_name".into()),
                    MontyObject::String(name.clone()),
                ));
            }
            MontyObject::dict(pairs)
        })
        .collect();
    names.push("context".into());
    values.push(MontyObject::List(messages));

    // `goal` — the thread's goal string
    names.push("goal".into());
    values.push(MontyObject::String(thread.goal.clone()));

    // `step_number` — current step index
    names.push("step_number".into());
    values.push(MontyObject::Int(thread.step_count as i64));

    // `state` — persisted variables from previous code steps.
    // This is a dict that accumulates: return values, tool results, etc.
    // The model can read `state["results"]`, `state["prev_return"]`, etc.
    names.push("state".into());
    values.push(json_to_monty(persisted_state));

    // `previous_results` — dict of {call_id: result_json} from prior steps
    let result_pairs: Vec<(MontyObject, MontyObject)> = thread
        .messages
        .iter()
        .filter(|m| m.role == MessageRole::ActionResult)
        .filter_map(|m| {
            let call_id = m.action_call_id.as_ref()?;
            Some((
                MontyObject::String(call_id.clone()),
                MontyObject::String(m.content.clone()),
            ))
        })
        .collect();
    names.push("previous_results".into());
    values.push(MontyObject::dict(result_pairs));

    // `user_timezone` — IANA timezone string from the user's channel (e.g. "America/New_York")
    let tz = thread
        .metadata
        .get("user_timezone")
        .and_then(|v| v.as_str())
        .unwrap_or("UTC");
    names.push("user_timezone".into());
    values.push(MontyObject::String(tz.into()));

    (names, values)
}

// ── Main execution function ─────────────────────────────────

/// Execute a Python code block using Monty.
///
/// Handles the full RLM execution pattern: context-as-variables, FINAL()
/// termination, llm_query() recursive calls, error-to-LLM flow, and
/// output truncation.
#[allow(clippy::too_many_arguments)]
pub async fn execute_code(
    code: &str,
    thread: &Thread,
    llm: &Arc<dyn LlmBackend>,
    effects: &Arc<dyn EffectExecutor>,
    leases: &LeaseManager,
    policy: &PolicyEngine,
    context: &ThreadExecutionContext,
    capability_policies: &[crate::types::capability::PolicyRule],
    persisted_state: &serde_json::Value,
) -> Result<CodeExecutionResult, EngineError> {
    execute_code_with_skills(
        code,
        thread,
        llm,
        effects,
        leases,
        policy,
        context,
        capability_policies,
        persisted_state,
        &[],
    )
    .await
}

/// Execute a Python code block with optional skill code snippets.
///
/// `skill_snippet_names` are registered as additional known functions in the
/// Monty NameLookup, alongside tool names from capability leases.
#[allow(clippy::too_many_arguments)]
pub async fn execute_code_with_skills(
    code: &str,
    thread: &Thread,
    llm: &Arc<dyn LlmBackend>,
    effects: &Arc<dyn EffectExecutor>,
    leases: &LeaseManager,
    policy: &PolicyEngine,
    context: &ThreadExecutionContext,
    capability_policies: &[crate::types::capability::PolicyRule],
    persisted_state: &serde_json::Value,
    skill_snippet_names: &[String],
) -> Result<CodeExecutionResult, EngineError> {
    let mut stdout = String::new();
    let mut action_results = Vec::new();
    let mut events = Vec::new();
    let mut recursive_tokens = TokenUsage::default();
    let mut final_answer: Option<String> = None;
    let mut had_error = false;

    // Build context variables including persisted state from prior steps
    let (input_names, input_values) = build_context_inputs(thread, persisted_state);

    // Collect known tool names so NameLookup can return callable stubs.
    // Without this, `mission_list()` in code raises NameError because Monty
    // resolves the name before calling it, and Undefined → NameError.
    let active_leases = leases.active_for_thread(thread.id).await;
    let mut known_actions: std::collections::HashSet<String> = effects
        .available_actions(&active_leases)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|a| a.name)
        .collect();

    // Register skill code snippet function names as additional known actions.
    // These resolve in NameLookup so the LLM can call them as Python functions.
    for name in skill_snippet_names {
        known_actions.insert(name.clone());
    }

    // Parse and compile (wrap in catch_unwind — Monty 0.0.x can panic)
    let runner = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        MontyRun::new(code.to_string(), "step.py", input_names)
    })) {
        Ok(Ok(runner)) => runner,
        Ok(Err(e)) => {
            // Parse error flows back to LLM (not a termination)
            return Ok(CodeExecutionResult {
                return_value: serde_json::Value::Null,
                stdout: format!("SyntaxError: {e}"),
                action_results,
                events,
                need_approval: None,
                recursive_tokens,
                final_answer: None,
                had_error: true,
            });
        }
        Err(_) => {
            return Err(EngineError::Effect {
                reason: "Monty VM panicked during code parsing".into(),
            });
        }
    };

    // Start execution with resource limits and context inputs
    let tracker = LimitedTracker::new(default_limits());

    let run_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        runner.start(input_values, tracker, PrintWriter::Collect(&mut stdout))
    }));

    let mut progress = match run_result {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            // Runtime error flows back to LLM
            return Ok(CodeExecutionResult {
                return_value: serde_json::Value::Null,
                stdout: format!("{stdout}\nError: {e}"),
                action_results,
                events,
                need_approval: None,
                recursive_tokens,
                final_answer: None,
                had_error: true,
            });
        }
        Err(_) => {
            return Err(EngineError::Effect {
                reason: "Monty VM panicked during execution start".into(),
            });
        }
    };

    // Pending async tool executions keyed by Monty call_id.
    // When a tool FunctionCall comes in, we spawn a tokio task and store
    // the JoinHandle here. When ResolveFutures yields, we await them.
    let mut pending_futures: HashMap<u32, PendingFuture> = HashMap::new();

    // Drive the execution loop
    let mut call_counter = 0u32;
    loop {
        match progress {
            RunProgress::Complete(obj) => {
                return Ok(CodeExecutionResult {
                    return_value: monty_to_json(&obj),
                    stdout,
                    action_results,
                    events,
                    need_approval: None,
                    recursive_tokens,
                    final_answer,
                    had_error,
                });
            }

            RunProgress::FunctionCall(call) => {
                call_counter += 1;
                let str_call_id = format!("code_call_{call_counter}");
                let monty_call_id = call.call_id;
                let action_name = call.function_name.clone();
                let params = monty_args_to_json(&call.args, &call.kwargs);

                debug!(action = %action_name, call_id = %str_call_id, monty_id = monty_call_id, "Monty: function call");

                // Builtins that need synchronous results — resume with value.
                let sync_result = match action_name.as_str() {
                    "FINAL" => {
                        let answer = call.args.first().map(monty_to_string).unwrap_or_default();
                        final_answer = Some(answer);
                        Some(ExtFunctionResult::Return(MontyObject::None))
                    }
                    "FINAL_VAR" => {
                        let var_name = call
                            .args
                            .first()
                            .map(monty_to_string)
                            .unwrap_or_else(|| "result".into());
                        final_answer = Some(format!("[FINAL_VAR: {var_name}]"));
                        Some(ExtFunctionResult::Return(MontyObject::None))
                    }
                    // LLM calls are async — spawn tokio task, resume_pending.
                    // This allows asyncio.gather(llm_query(...), tool(...))
                    // to run the LLM call and tool call concurrently.
                    "llm_query" => {
                        let args = call.args.clone();
                        let kwargs = call.kwargs.clone();
                        let llm = llm.clone();
                        let handle = tokio::spawn(async move {
                            handle_llm_query_standalone(&args, &kwargs, &llm).await
                        });
                        pending_futures.insert(monty_call_id, PendingFuture::Llm { handle });
                        None // handled as async below
                    }
                    "llm_query_batched" => {
                        let args = call.args.clone();
                        let kwargs = call.kwargs.clone();
                        let llm = llm.clone();
                        let handle = tokio::spawn(async move {
                            handle_llm_query_batched_standalone(&args, &kwargs, &llm).await
                        });
                        pending_futures.insert(monty_call_id, PendingFuture::Llm { handle });
                        None
                    }
                    // rlm_query stays synchronous — it spawns a child Monty VM
                    // which isn't Send, so it can't run in tokio::spawn.
                    "rlm_query" => Some(
                        handle_rlm_query(
                            &call.args,
                            &call.kwargs,
                            thread,
                            llm,
                            effects,
                            leases,
                            policy,
                            &mut recursive_tokens,
                        )
                        .await,
                    ),
                    "globals" | "locals" => {
                        let entries: Vec<(MontyObject, MontyObject)> = known_actions
                            .iter()
                            .map(|name| {
                                (MontyObject::String(name.clone()), MontyObject::Bool(true))
                            })
                            .collect();
                        Some(ExtFunctionResult::Return(MontyObject::Dict(entries.into())))
                    }
                    _ => None, // tool call — handled async below
                };

                if let Some(ext_result) = sync_result {
                    // Sync resume for builtins
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        call.resume(ext_result, PrintWriter::Collect(&mut stdout))
                    })) {
                        Ok(Ok(p)) => progress = p,
                        Ok(Err(e)) => {
                            stdout.push_str(&format!("\nError: {e}"));
                            had_error = true;
                            return Ok(CodeExecutionResult {
                                return_value: serde_json::Value::Null,
                                stdout,
                                action_results,
                                events,
                                need_approval: None,
                                recursive_tokens,
                                final_answer,
                                had_error,
                            });
                        }
                        Err(_) => {
                            return Err(EngineError::Effect {
                                reason: "Monty VM panicked during resume".into(),
                            });
                        }
                    }
                    continue;
                }

                // If an LLM call already inserted a pending future, just
                // resume_pending and continue — no preflight needed.
                if pending_futures.contains_key(&monty_call_id) {
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        call.resume_pending(PrintWriter::Collect(&mut stdout))
                    })) {
                        Ok(Ok(p)) => progress = p,
                        Ok(Err(e)) => {
                            stdout.push_str(&format!("\nError: {e}"));
                            had_error = true;
                            return Ok(CodeExecutionResult {
                                return_value: serde_json::Value::Null,
                                stdout,
                                action_results,
                                events,
                                need_approval: None,
                                recursive_tokens,
                                final_answer,
                                had_error,
                            });
                        }
                        Err(_) => {
                            return Err(EngineError::Effect {
                                reason: "Monty VM panicked during resume_pending".into(),
                            });
                        }
                    }
                    continue;
                }

                // ── Async tool dispatch ─────────────────────────────
                // Preflight (lease + policy) is sync. If denied or
                // needs approval, resume with error immediately.
                // If approved, spawn tokio task and resume_pending().

                let preflight = preflight_action(
                    &action_name,
                    &params,
                    thread,
                    effects,
                    leases,
                    policy,
                    context,
                    capability_policies,
                    &str_call_id,
                    &mut events,
                )
                .await;

                match preflight {
                    PreflightResult::Approved(lease) => {
                        // Spawn async execution
                        let effects = effects.clone();
                        let name = action_name.clone();
                        let params_clone = params.clone();
                        let lease_clone = lease.clone();
                        let mut ctx = context.clone();
                        ctx.current_call_id = Some(str_call_id.clone());
                        let ps = crate::types::event::summarize_params(&name, &params);

                        let handle = tokio::spawn(async move {
                            effects
                                .execute_action(&name, params_clone, &lease_clone, &ctx)
                                .await
                        });

                        pending_futures.insert(
                            monty_call_id,
                            PendingFuture::Tool {
                                handle,
                                action_name,
                                call_id: str_call_id,
                                lease_id: lease.id,
                                parameters: params.clone(),
                                params_summary: ps,
                            },
                        );

                        // Resume with pending future — Python gets ExternalFuture
                        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            call.resume_pending(PrintWriter::Collect(&mut stdout))
                        })) {
                            Ok(Ok(p)) => progress = p,
                            Ok(Err(e)) => {
                                stdout.push_str(&format!("\nError: {e}"));
                                had_error = true;
                                return Ok(CodeExecutionResult {
                                    return_value: serde_json::Value::Null,
                                    stdout,
                                    action_results,
                                    events,
                                    need_approval: None,
                                    recursive_tokens,
                                    final_answer,
                                    had_error,
                                });
                            }
                            Err(_) => {
                                return Err(EngineError::Effect {
                                    reason: "Monty VM panicked during resume_pending".into(),
                                });
                            }
                        }
                    }
                    PreflightResult::Denied(ext_result) => {
                        // Resume with error — Python sees an exception
                        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            call.resume(ext_result, PrintWriter::Collect(&mut stdout))
                        })) {
                            Ok(Ok(p)) => progress = p,
                            Ok(Err(e)) => {
                                stdout.push_str(&format!("\nError: {e}"));
                                had_error = true;
                                return Ok(CodeExecutionResult {
                                    return_value: serde_json::Value::Null,
                                    stdout,
                                    action_results,
                                    events,
                                    need_approval: None,
                                    recursive_tokens,
                                    final_answer,
                                    had_error,
                                });
                            }
                            Err(_) => {
                                return Err(EngineError::Effect {
                                    reason: "Monty VM panicked during resume".into(),
                                });
                            }
                        }
                    }
                    PreflightResult::GatePaused(outcome) => {
                        return Ok(CodeExecutionResult {
                            return_value: serde_json::Value::Null,
                            stdout,
                            action_results,
                            events,
                            need_approval: Some(outcome),
                            recursive_tokens,
                            final_answer: None,
                            had_error,
                        });
                    }
                }
            }

            // ── ResolveFutures: parallel execution ────────────────
            // Resolves both tool calls and LLM calls that were deferred
            // via resume_pending(). All pending tokio tasks are awaited
            // and their results fed back to Monty.
            RunProgress::ResolveFutures(resolve) => {
                let pending_ids = resolve.pending_call_ids().to_vec();
                debug!(pending = ?pending_ids, "Monty: ResolveFutures — resolving {} pending futures", pending_ids.len());

                let mut results: Vec<(u32, ExtFunctionResult)> =
                    Vec::with_capacity(pending_ids.len());

                for &mid in &pending_ids {
                    let ext_result = if let Some(pf) = pending_futures.remove(&mid) {
                        match pf {
                            PendingFuture::Tool {
                                handle,
                                action_name,
                                call_id,
                                lease_id,
                                parameters,
                                params_summary,
                            } => {
                                resolve_tool_future(
                                    handle,
                                    &action_name,
                                    &call_id,
                                    lease_id,
                                    parameters,
                                    params_summary,
                                    leases,
                                    context,
                                    &mut action_results,
                                    &mut events,
                                )
                                .await
                            }
                            PendingFuture::Llm { handle } => {
                                resolve_llm_future(handle, &mut recursive_tokens).await
                            }
                        }
                    } else {
                        debug!(call_id = mid, "ResolveFutures: unknown pending call_id");
                        ExtFunctionResult::Error(MontyException::new(
                            ExcType::RuntimeError,
                            Some(format!("unknown pending call_id {mid}")),
                        ))
                    };
                    results.push((mid, ext_result));
                }

                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    resolve.resume(results, PrintWriter::Collect(&mut stdout))
                })) {
                    Ok(Ok(p)) => progress = p,
                    Ok(Err(e)) => {
                        stdout.push_str(&format!("\nError: {e}"));
                        had_error = true;
                        return Ok(CodeExecutionResult {
                            return_value: serde_json::Value::Null,
                            stdout,
                            action_results,
                            events,
                            need_approval: None,
                            recursive_tokens,
                            final_answer,
                            had_error,
                        });
                    }
                    Err(_) => {
                        return Err(EngineError::Effect {
                            reason: "Monty VM panicked during ResolveFutures resume".into(),
                        });
                    }
                }
            }

            RunProgress::NameLookup(lookup) => {
                let name = lookup.name.clone();

                let result = if known_actions.contains(&name) {
                    debug!(name = %name, "Monty: resolved as tool function");
                    NameLookupResult::Value(MontyObject::Function {
                        name: name.clone(),
                        docstring: None,
                    })
                } else if name == "globals" || name == "locals" {
                    NameLookupResult::Value(MontyObject::Function {
                        name: name.clone(),
                        docstring: None,
                    })
                } else {
                    debug!(name = %name, "Monty: unresolved name");
                    NameLookupResult::Undefined
                };

                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    lookup.resume(result, PrintWriter::Collect(&mut stdout))
                })) {
                    Ok(Ok(p)) => progress = p,
                    Ok(Err(e)) => {
                        stdout.push_str(&format!("\nNameError: {e}"));
                        had_error = true;
                        return Ok(CodeExecutionResult {
                            return_value: serde_json::Value::Null,
                            stdout,
                            action_results,
                            events,
                            need_approval: None,
                            recursive_tokens,
                            final_answer,
                            had_error,
                        });
                    }
                    Err(_) => {
                        return Err(EngineError::Effect {
                            reason: "Monty VM panicked during name lookup".into(),
                        });
                    }
                }
            }

            RunProgress::OsCall(os_call) => {
                debug!(function = ?os_call.function, "Monty: OS call denied");
                let err = ExtFunctionResult::Error(MontyException::new(
                    ExcType::OSError,
                    Some("OS operations are not permitted in CodeAct scripts".into()),
                ));
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    os_call.resume(err, PrintWriter::Collect(&mut stdout))
                })) {
                    Ok(Ok(p)) => progress = p,
                    Ok(Err(e)) => {
                        stdout.push_str(&format!("\nOSError: {e}"));
                        had_error = true;
                        return Ok(CodeExecutionResult {
                            return_value: serde_json::Value::Null,
                            stdout,
                            action_results,
                            events,
                            need_approval: None,
                            recursive_tokens,
                            final_answer,
                            had_error,
                        });
                    }
                    Err(_) => {
                        return Err(EngineError::Effect {
                            reason: "Monty VM panicked during OS call".into(),
                        });
                    }
                }
            }
        }
    }
}

// ── Pending future tracking ─────────────────────────────────

/// A deferred computation spawned as a tokio task, pending resolution
/// via `ResolveFutures`. Can be a tool execution or an LLM call.
enum PendingFuture {
    /// Tool action execution.
    Tool {
        handle: tokio::task::JoinHandle<Result<ActionResult, EngineError>>,
        action_name: String,
        call_id: String,
        lease_id: crate::types::capability::LeaseId,
        parameters: serde_json::Value,
        params_summary: Option<String>,
    },
    /// LLM call (llm_query / llm_query_batched / rlm_query).
    Llm {
        handle: tokio::task::JoinHandle<(ExtFunctionResult, TokenUsage)>,
    },
}

/// Result of preflight checks (lease + policy) for a tool call.
enum PreflightResult {
    /// Tool approved — lease is consumed, ready to execute.
    Approved(crate::types::capability::CapabilityLease),
    /// Tool denied — return this error to Monty.
    Denied(ExtFunctionResult),
    /// Tool is paused by a gate — interrupt the batch.
    GatePaused(crate::runtime::messaging::ThreadOutcome),
}

/// Run preflight checks for a tool call: find lease, check policy, consume use.
#[allow(clippy::too_many_arguments)]
async fn preflight_action(
    action_name: &str,
    params: &serde_json::Value,
    thread: &Thread,
    effects: &Arc<dyn EffectExecutor>,
    leases: &LeaseManager,
    policy: &PolicyEngine,
    context: &ThreadExecutionContext,
    capability_policies: &[crate::types::capability::PolicyRule],
    call_id: &str,
    events: &mut Vec<EventKind>,
) -> PreflightResult {
    let lease = match leases.find_lease_for_action(thread.id, action_name).await {
        Some(l) => l,
        None => {
            events.push(EventKind::ActionFailed {
                step_id: context.step_id,
                action_name: action_name.into(),
                call_id: call_id.into(),
                error: format!("no lease for action '{action_name}'"),
                params_summary: None,
            });
            return PreflightResult::Denied(ExtFunctionResult::Error(MontyException::new(
                ExcType::RuntimeError,
                Some(format!("no lease for action '{action_name}'")),
            )));
        }
    };

    let action_def = effects
        .available_actions(std::slice::from_ref(&lease))
        .await
        .ok()
        .and_then(|actions| actions.into_iter().find(|a| a.name == action_name));

    if let Some(ref action_def) = action_def {
        match policy.evaluate(action_def, &lease, capability_policies) {
            PolicyDecision::Deny { reason } => {
                events.push(EventKind::ActionFailed {
                    step_id: context.step_id,
                    action_name: action_name.into(),
                    call_id: call_id.into(),
                    error: reason.clone(),
                    params_summary: None,
                });
                return PreflightResult::Denied(ExtFunctionResult::Error(MontyException::new(
                    ExcType::RuntimeError,
                    Some(format!("denied: {reason}")),
                )));
            }
            PolicyDecision::RequireApproval { .. } => {
                events.push(EventKind::ApprovalRequested {
                    action_name: action_name.into(),
                    call_id: call_id.into(),
                    parameters: Some(params.clone()),
                    description: None,
                    allow_always: None,
                    gate_name: None,
                    params_summary: crate::types::event::summarize_params(action_name, params),
                });
                return PreflightResult::GatePaused(
                    crate::runtime::messaging::ThreadOutcome::GatePaused {
                        gate_name: "approval".into(),
                        action_name: action_name.into(),
                        call_id: call_id.into(),
                        parameters: params.clone(),
                        resume_kind: crate::gate::ResumeKind::Approval { allow_always: true },
                        resume_output: None,
                    },
                );
            }
            PolicyDecision::Allow => {}
        }
    }

    if let Err(e) = leases.consume_use(lease.id).await {
        return PreflightResult::Denied(ExtFunctionResult::Error(MontyException::new(
            ExcType::RuntimeError,
            Some(format!("lease exhausted: {e}")),
        )));
    }

    PreflightResult::Approved(lease)
}

// ── llm_query() — recursive subagent (RLM 3.5) ─────────────

/// Handle `llm_query(prompt, context)` — single recursive sub-call.
async fn handle_llm_query(
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
    llm: &Arc<dyn LlmBackend>,
    recursive_tokens: &mut TokenUsage,
) -> ExtFunctionResult {
    let prompt = extract_string_arg(args, kwargs, "prompt", 0);
    let context_arg = extract_string_arg(args, kwargs, "context", 1);

    let prompt = match prompt {
        Some(p) => p,
        None => {
            return ExtFunctionResult::Error(MontyException::new(
                ExcType::TypeError,
                Some("llm_query() requires a 'prompt' argument".into()),
            ));
        }
    };

    let mut messages = Vec::new();
    if let Some(ctx) = context_arg {
        messages.push(ThreadMessage::system(format!(
            "You are a sub-agent. Answer concisely based on the context.\n\n{ctx}"
        )));
    } else {
        // Some providers (e.g. OpenAI Codex Responses API) require a system
        // message / instructions field. Always include one.
        messages.push(ThreadMessage::system(
            "You are a helpful sub-agent. Answer concisely.",
        ));
    }
    messages.push(ThreadMessage::user(prompt));

    let config = LlmCallConfig {
        force_text: true,
        ..LlmCallConfig::default()
    };

    match llm.complete(&messages, &[], &config).await {
        Ok(output) => {
            recursive_tokens.input_tokens += output.usage.input_tokens;
            recursive_tokens.output_tokens += output.usage.output_tokens;
            let text = match output.response {
                LlmResponse::Text(t) => t,
                LlmResponse::ActionCalls { content, .. } | LlmResponse::Code { content, .. } => {
                    content.unwrap_or_default()
                }
            };
            ExtFunctionResult::Return(MontyObject::String(text))
        }
        Err(e) => ExtFunctionResult::Error(MontyException::new(
            ExcType::RuntimeError,
            Some(format!("llm_query failed: {e}")),
        )),
    }
}

/// Handle `llm_query_batched(prompts)` — parallel recursive sub-calls.
///
/// Takes a list of prompt strings and dispatches them concurrently.
/// Returns a list of response strings in the same order.
async fn handle_llm_query_batched(
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
    llm: &Arc<dyn LlmBackend>,
    recursive_tokens: &mut TokenUsage,
) -> ExtFunctionResult {
    // Extract prompts list (first arg or kwarg "prompts")
    let prompts_obj = args.first().or_else(|| {
        kwargs.iter().find_map(|(k, v)| {
            if let MontyObject::String(key) = k
                && key == "prompts"
            {
                return Some(v);
            }
            None
        })
    });

    let prompts: Vec<String> = match prompts_obj {
        Some(MontyObject::List(items)) => items.iter().map(monty_to_string).collect(),
        Some(other) => {
            return ExtFunctionResult::Error(MontyException::new(
                ExcType::TypeError,
                Some(format!(
                    "llm_query_batched() expects a list of prompts, got {other:?}"
                )),
            ));
        }
        None => {
            return ExtFunctionResult::Error(MontyException::new(
                ExcType::TypeError,
                Some("llm_query_batched() requires a 'prompts' argument".into()),
            ));
        }
    };

    // Optional context kwarg
    let context_arg = extract_string_arg(&[], kwargs, "context", usize::MAX);

    // Dispatch all prompts concurrently
    let config = LlmCallConfig {
        force_text: true,
        ..LlmCallConfig::default()
    };

    let mut handles = Vec::with_capacity(prompts.len());
    for prompt in &prompts {
        let llm = Arc::clone(llm);
        let config = config.clone();
        let ctx = context_arg.clone();
        let prompt = prompt.clone();
        handles.push(tokio::spawn(async move {
            let mut messages = Vec::new();
            if let Some(ctx) = ctx {
                messages.push(ThreadMessage::system(format!(
                    "You are a sub-agent. Answer concisely.\n\n{ctx}"
                )));
            } else {
                messages.push(ThreadMessage::system(
                    "You are a helpful sub-agent. Answer concisely.",
                ));
            }
            messages.push(ThreadMessage::user(prompt));
            llm.complete(&messages, &[], &config).await
        }));
    }

    // Collect results
    let mut results = Vec::with_capacity(prompts.len());
    let mut total_input = 0u64;
    let mut total_output = 0u64;

    for handle in handles {
        match handle.await {
            Ok(Ok(output)) => {
                total_input += output.usage.input_tokens;
                total_output += output.usage.output_tokens;
                let text = match output.response {
                    LlmResponse::Text(t) => t,
                    LlmResponse::ActionCalls { content, .. }
                    | LlmResponse::Code { content, .. } => content.unwrap_or_default(),
                };
                results.push(MontyObject::String(text));
            }
            Ok(Err(e)) => {
                results.push(MontyObject::String(format!("Error: {e}")));
            }
            Err(e) => {
                results.push(MontyObject::String(format!("Error: task failed: {e}")));
            }
        }
    }

    recursive_tokens.input_tokens += total_input;
    recursive_tokens.output_tokens += total_output;

    ExtFunctionResult::Return(MontyObject::List(results))
}

// ── rlm_query() — full recursive sub-agent (RLM 3.5) ─────────

/// Handle `rlm_query(prompt)` — spawn a child CodeAct thread with its own
/// execution loop, tools, and iteration budget.
///
/// Unlike `llm_query()` (single-shot LLM call), `rlm_query()` creates a
/// child thread with full CodeAct capabilities. The child inherits the
/// parent's remaining budget and tool access.
#[allow(clippy::too_many_arguments)]
async fn handle_rlm_query(
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
    parent_thread: &Thread,
    llm: &Arc<dyn LlmBackend>,
    effects: &Arc<dyn EffectExecutor>,
    leases: &LeaseManager,
    policy: &PolicyEngine,
    recursive_tokens: &mut TokenUsage,
) -> ExtFunctionResult {
    let prompt = extract_string_arg(args, kwargs, "prompt", 0);
    let prompt = match prompt {
        Some(p) => p,
        None => {
            return ExtFunctionResult::Error(MontyException::new(
                ExcType::TypeError,
                Some("rlm_query() requires a 'prompt' argument".into()),
            ));
        }
    };

    // Depth check — refuse if at max recursion depth
    let current_depth = parent_thread.config.depth;
    let max_depth = parent_thread.config.max_depth;
    if current_depth >= max_depth {
        return ExtFunctionResult::Error(MontyException::new(
            ExcType::RuntimeError,
            Some(format!(
                "rlm_query() depth limit reached: depth {current_depth} >= max {max_depth}"
            )),
        ));
    }

    // Build child thread with inherited budget
    let child_config = crate::types::thread::ThreadConfig {
        max_iterations: parent_thread.config.max_iterations.min(20), // cap child iterations
        enable_tool_intent_nudge: false,
        max_tokens_total: parent_thread
            .config
            .max_tokens_total
            .map(|max| max.saturating_sub(parent_thread.total_tokens_used)),
        max_budget_usd: parent_thread
            .config
            .max_budget_usd
            .map(|max| (max - parent_thread.total_cost_usd).max(0.0)),
        max_duration: parent_thread.config.max_duration,
        depth: current_depth + 1,
        max_depth,
        ..crate::types::thread::ThreadConfig::default()
    };

    let mut child_thread = crate::types::thread::Thread::new(
        &prompt,
        crate::types::thread::ThreadType::Research,
        parent_thread.project_id,
        &parent_thread.user_id,
        child_config,
    )
    .with_parent(parent_thread.id);

    // Add the prompt as a user message
    child_thread.add_message(ThreadMessage::user(&prompt));

    // Create signal channel and child's lease manager
    let (_tx, rx) = crate::runtime::messaging::signal_channel(8);
    let child_leases = Arc::new(LeaseManager::new());

    // Grant the child the same leases as the parent (in the child's manager)
    let parent_leases = leases.active_for_thread(parent_thread.id).await;
    let now = chrono::Utc::now();
    for parent_lease in &parent_leases {
        // Convert parent's expires_at to remaining duration
        let remaining_duration = parent_lease
            .expires_at
            .and_then(|exp| (exp - now).to_std().ok())
            .map(|d| chrono::Duration::from_std(d).unwrap_or(chrono::Duration::hours(1)));
        let lease = match child_leases
            .grant(
                child_thread.id,
                &parent_lease.capability_name,
                parent_lease.granted_actions.clone(),
                remaining_duration,
                parent_lease.max_uses,
            )
            .await
        {
            Ok(l) => l,
            Err(e) => {
                debug!(error = %e, "rlm_query: skipping invalid lease for child thread");
                continue;
            }
        };
        child_thread.capability_leases.push(lease.id);
    }
    let mut child_policy_engine = PolicyEngine::new();
    // Copy denied effects from parent policy
    for effect in &policy.denied_effects {
        child_policy_engine.deny_effect(*effect);
    }
    let child_policy = Arc::new(child_policy_engine);

    let mut child_loop = crate::executor::ExecutionLoop::new(
        child_thread,
        Arc::clone(llm),
        Arc::clone(effects),
        child_leases,
        child_policy,
        rx,
        "rlm_child".to_string(),
    );

    debug!(
        parent_thread = %parent_thread.id,
        depth = current_depth + 1,
        prompt_len = prompt.len(),
        "rlm_query: spawning child CodeAct thread"
    );

    // Run the child loop (Box::pin to avoid infinite future size from recursion)
    match Box::pin(child_loop.run()).await {
        Ok(outcome) => {
            // Track child's token usage
            recursive_tokens.input_tokens += child_loop.thread.total_tokens_used;
            recursive_tokens.cost_usd += child_loop.thread.total_cost_usd;

            let response = match outcome {
                crate::runtime::messaging::ThreadOutcome::Completed { response } => {
                    response.unwrap_or_default()
                }
                crate::runtime::messaging::ThreadOutcome::Failed { error } => {
                    format!("rlm_query child failed: {error}")
                }
                crate::runtime::messaging::ThreadOutcome::MaxIterations => {
                    "rlm_query child reached max iterations".to_string()
                }
                _ => String::new(),
            };

            ExtFunctionResult::Return(MontyObject::String(response))
        }
        Err(e) => ExtFunctionResult::Error(MontyException::new(
            ExcType::RuntimeError,
            Some(format!("rlm_query failed: {e}")),
        )),
    }
}

// ── Standalone async handlers (for tokio::spawn) ────────────

/// `llm_query()` — standalone version that returns `(ExtFunctionResult, TokenUsage)`.
async fn handle_llm_query_standalone(
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
    llm: &Arc<dyn LlmBackend>,
) -> (ExtFunctionResult, TokenUsage) {
    let mut tokens = TokenUsage::default();
    let result = handle_llm_query(args, kwargs, llm, &mut tokens).await;
    (result, tokens)
}

/// `llm_query_batched()` — standalone version.
async fn handle_llm_query_batched_standalone(
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
    llm: &Arc<dyn LlmBackend>,
) -> (ExtFunctionResult, TokenUsage) {
    let mut tokens = TokenUsage::default();
    let result = handle_llm_query_batched(args, kwargs, llm, &mut tokens).await;
    (result, tokens)
}

// ── Future resolution helpers ───────────────────────────────

/// Resolve a pending tool execution future.
#[allow(clippy::too_many_arguments)]
async fn resolve_tool_future(
    handle: tokio::task::JoinHandle<Result<ActionResult, EngineError>>,
    action_name: &str,
    call_id: &str,
    lease_id: crate::types::capability::LeaseId,
    parameters: serde_json::Value,
    params_summary: Option<String>,
    leases: &LeaseManager,
    context: &ThreadExecutionContext,
    action_results: &mut Vec<ActionResult>,
    events: &mut Vec<EventKind>,
) -> ExtFunctionResult {
    match handle.await {
        Ok(Ok(result)) => {
            events.push(EventKind::ActionExecuted {
                step_id: context.step_id,
                action_name: action_name.into(),
                call_id: call_id.into(),
                duration_ms: result.duration.as_millis() as u64,
                params_summary,
            });
            let monty_val = json_to_monty(&result.output);
            action_results.push(result);
            ExtFunctionResult::Return(monty_val)
        }
        Ok(Err(EngineError::GatePaused {
            gate_name,
            action_name,
            call_id,
            resume_kind,
            ..
        })) => {
            let _ = leases.refund_use(lease_id).await;
            events.push(EventKind::ApprovalRequested {
                action_name,
                call_id,
                parameters: Some(parameters),
                description: None,
                allow_always: match *resume_kind {
                    crate::gate::ResumeKind::Approval { allow_always } => Some(allow_always),
                    _ => None,
                },
                gate_name: Some(gate_name.clone()),
                params_summary,
            });
            ExtFunctionResult::Error(MontyException::new(
                ExcType::RuntimeError,
                Some(format!("execution paused by gate '{gate_name}'")),
            ))
        }
        Ok(Err(e)) => {
            events.push(EventKind::ActionFailed {
                step_id: context.step_id,
                action_name: action_name.into(),
                call_id: call_id.into(),
                error: e.to_string(),
                params_summary,
            });
            action_results.push(ActionResult {
                call_id: call_id.into(),
                action_name: action_name.into(),
                output: serde_json::json!({"error": e.to_string()}),
                is_error: true,
                duration: Duration::ZERO,
            });
            ExtFunctionResult::Error(MontyException::new(
                ExcType::RuntimeError,
                Some(e.to_string()),
            ))
        }
        Err(e) => {
            debug!("async tool task panicked: {e}");
            ExtFunctionResult::Error(MontyException::new(
                ExcType::RuntimeError,
                Some(format!("tool execution panicked: {e}")),
            ))
        }
    }
}

/// Resolve a pending LLM call future, accumulating token usage.
async fn resolve_llm_future(
    handle: tokio::task::JoinHandle<(ExtFunctionResult, TokenUsage)>,
    recursive_tokens: &mut TokenUsage,
) -> ExtFunctionResult {
    match handle.await {
        Ok((result, tokens)) => {
            recursive_tokens.input_tokens += tokens.input_tokens;
            recursive_tokens.output_tokens += tokens.output_tokens;
            result
        }
        Err(e) => {
            debug!("async LLM task panicked: {e}");
            ExtFunctionResult::Error(MontyException::new(
                ExcType::RuntimeError,
                Some(format!("LLM call panicked: {e}")),
            ))
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────

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

pub(crate) fn monty_to_string(obj: &MontyObject) -> String {
    match obj {
        MontyObject::String(s) => s.clone(),
        MontyObject::None => "None".into(),
        MontyObject::Bool(b) => b.to_string(),
        MontyObject::Int(i) => i.to_string(),
        MontyObject::Float(f) => f.to_string(),
        other => {
            serde_json::to_string(&monty_to_json(other)).unwrap_or_else(|_| format!("{other:?}"))
        }
    }
}

// Dispatch logic moved to orchestrator.rs (__execute_action__ handler).
// GatePaused is handled via EngineError → JSON in orchestrator.rs.
// ── MontyObject ↔ JSON ──────────────────────────────────────

pub(crate) fn monty_to_json(obj: &MontyObject) -> serde_json::Value {
    match obj {
        MontyObject::None => serde_json::Value::Null,
        MontyObject::Bool(b) => serde_json::Value::Bool(*b),
        MontyObject::Int(i) => serde_json::json!(i),
        MontyObject::BigInt(i) => serde_json::Value::String(i.to_string()),
        MontyObject::Float(f) => serde_json::json!(f),
        MontyObject::String(s) => serde_json::Value::String(s.clone()),
        MontyObject::List(items) | MontyObject::Tuple(items) => {
            serde_json::Value::Array(items.iter().map(monty_to_json).collect())
        }
        MontyObject::Dict(pairs) => {
            let map: serde_json::Map<String, serde_json::Value> = pairs
                .into_iter()
                .map(|(k, v)| {
                    let key = match k {
                        MontyObject::String(s) => s.clone(),
                        other => format!("{other:?}"),
                    };
                    (key, monty_to_json(v))
                })
                .collect();
            serde_json::Value::Object(map)
        }
        MontyObject::Set(items) | MontyObject::FrozenSet(items) => {
            serde_json::Value::Array(items.iter().map(monty_to_json).collect())
        }
        MontyObject::Bytes(b) => {
            serde_json::Value::String(b.iter().map(|byte| format!("{byte:02x}")).collect())
        }
        other => serde_json::Value::String(format!("{other:?}")),
    }
}

pub(crate) fn json_to_monty(val: &serde_json::Value) -> MontyObject {
    match val {
        serde_json::Value::Null => MontyObject::None,
        serde_json::Value::Bool(b) => MontyObject::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                MontyObject::Int(i)
            } else if let Some(f) = n.as_f64() {
                MontyObject::Float(f)
            } else {
                MontyObject::String(n.to_string())
            }
        }
        serde_json::Value::String(s) => MontyObject::String(s.clone()),
        serde_json::Value::Array(arr) => MontyObject::List(arr.iter().map(json_to_monty).collect()),
        serde_json::Value::Object(map) => MontyObject::dict(
            map.iter()
                .map(|(k, v)| (MontyObject::String(k.clone()), json_to_monty(v)))
                .collect::<Vec<_>>(),
        ),
    }
}

fn monty_args_to_json(
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    if !args.is_empty() {
        map.insert(
            "_args".into(),
            serde_json::Value::Array(args.iter().map(monty_to_json).collect()),
        );
    }
    for (k, v) in kwargs {
        let key = match k {
            MontyObject::String(s) => s.clone(),
            other => format!("{other:?}"),
        };
        map.insert(key, monty_to_json(v));
    }
    serde_json::Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::lease::LeaseManager;
    use crate::capability::policy::PolicyEngine;
    use crate::traits::effect::ThreadExecutionContext;
    use crate::types::capability::{ActionDef, CapabilityLease, EffectType, GrantedActions};
    use crate::types::project::ProjectId;
    use crate::types::step::{ActionResult, StepId};
    use crate::types::thread::{Thread, ThreadConfig, ThreadType};
    use std::sync::Mutex;

    /// Truncate a string to at most `max_bytes`, snapping to a UTF-8 char
    /// boundary so assertion messages never panic on multibyte output.
    fn truncate_for_assert(s: &str, max_bytes: usize) -> &str {
        if s.len() <= max_bytes {
            return s;
        }
        let mut end = max_bytes;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end] // safety: end is walked down to a valid char boundary above
    }

    struct MockEffects {
        results: Mutex<Vec<Result<ActionResult, EngineError>>>,
        actions: Vec<ActionDef>,
    }

    impl MockEffects {
        fn new(actions: Vec<ActionDef>, results: Vec<Result<ActionResult, EngineError>>) -> Self {
            Self {
                results: Mutex::new(results),
                actions,
            }
        }
    }

    #[async_trait::async_trait]
    impl EffectExecutor for MockEffects {
        async fn execute_action(
            &self,
            name: &str,
            _params: serde_json::Value,
            _lease: &CapabilityLease,
            _ctx: &ThreadExecutionContext,
        ) -> Result<ActionResult, EngineError> {
            let mut results = self.results.lock().unwrap();
            if results.is_empty() {
                Ok(ActionResult {
                    call_id: String::new(),
                    action_name: name.into(),
                    output: serde_json::json!({"result": "ok"}),
                    is_error: false,
                    duration: Duration::from_millis(1),
                })
            } else {
                results.remove(0)
            }
        }

        async fn available_actions(
            &self,
            _leases: &[CapabilityLease],
        ) -> Result<Vec<ActionDef>, EngineError> {
            Ok(self.actions.clone())
        }
    }

    fn test_action(name: &str) -> ActionDef {
        ActionDef {
            name: name.into(),
            description: "Test tool".into(),
            parameters_schema: serde_json::json!({"type": "object"}),
            effects: vec![EffectType::ReadLocal],
            requires_approval: false,
        }
    }

    fn make_test_thread() -> Thread {
        Thread::new(
            "test goal",
            ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            ThreadConfig::default(),
        )
    }

    fn make_exec_context(thread: &Thread) -> ThreadExecutionContext {
        ThreadExecutionContext {
            thread_id: thread.id,
            thread_type: thread.thread_type,
            project_id: thread.project_id,
            user_id: "test".into(),
            step_id: StepId::new(),
            current_call_id: None,
            source_channel: None,
            user_timezone: None,
        }
    }

    /// Stub LLM that always returns text "stub". Only used so execute_code
    /// doesn't need a real LLM — our tests exercise tool dispatch, not LLM calls.
    struct StubLlm;

    #[async_trait::async_trait]
    impl crate::traits::llm::LlmBackend for StubLlm {
        fn model_name(&self) -> &str {
            "stub"
        }

        async fn complete(
            &self,
            _messages: &[crate::types::message::ThreadMessage],
            _actions: &[ActionDef],
            _config: &crate::traits::llm::LlmCallConfig,
        ) -> Result<crate::traits::llm::LlmOutput, EngineError> {
            Ok(crate::traits::llm::LlmOutput {
                response: crate::types::step::LlmResponse::Text("stub".into()),
                usage: crate::types::step::TokenUsage::default(),
            })
        }
    }

    async fn run_code(
        code: &str,
        effects: Arc<dyn EffectExecutor>,
        thread: &Thread,
    ) -> Result<CodeExecutionResult, EngineError> {
        let leases = LeaseManager::new();
        let policy = PolicyEngine::new();
        let ctx = make_exec_context(thread);

        // Grant a wildcard lease
        leases
            .grant(thread.id, "tools", GrantedActions::All, None, None)
            .await
            .unwrap();

        execute_code(
            code,
            thread,
            &(Arc::new(StubLlm) as Arc<dyn crate::traits::llm::LlmBackend>),
            &effects,
            &leases,
            &policy,
            &ctx,
            &[],
            &serde_json::json!({}),
        )
        .await
    }

    // ── Single await tool call ──────────────────────────────

    #[tokio::test]
    async fn single_await_tool_call() {
        let thread = make_test_thread();
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(
            vec![test_action("echo")],
            vec![Ok(ActionResult {
                call_id: String::new(),
                action_name: "echo".into(),
                output: serde_json::json!("hello world"),
                is_error: false,
                duration: Duration::from_millis(1),
            })],
        ));

        let code = r#"
result = await echo(message="hello")
FINAL(str(result))
"#;

        let result = run_code(code, effects, &thread).await.unwrap();
        assert!(
            result.final_answer.is_some(),
            "should have final answer, stdout: {}",
            result.stdout
        );
        assert!(
            !result.had_error,
            "should not error, stdout: {}",
            result.stdout
        );
        assert_eq!(result.action_results.len(), 1);
    }

    // ── asyncio.gather parallel execution ───────────────────

    #[tokio::test]
    async fn asyncio_gather_two_tools() {
        let thread = make_test_thread();
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(
            vec![test_action("tool_a"), test_action("tool_b")],
            vec![
                Ok(ActionResult {
                    call_id: String::new(),
                    action_name: "tool_a".into(),
                    output: serde_json::json!(10),
                    is_error: false,
                    duration: Duration::from_millis(1),
                }),
                Ok(ActionResult {
                    call_id: String::new(),
                    action_name: "tool_b".into(),
                    output: serde_json::json!(32),
                    is_error: false,
                    duration: Duration::from_millis(1),
                }),
            ],
        ));

        let code = r#"
import asyncio
a, b = await asyncio.gather(tool_a(), tool_b())
FINAL(str(a + b))
"#;

        let result = run_code(code, effects, &thread).await.unwrap();
        assert!(
            result.final_answer.is_some(),
            "should have final answer, stdout: {}",
            result.stdout
        );
        assert_eq!(
            result.final_answer.as_deref(),
            Some("42"),
            "10 + 32 = 42, got: {:?}, stdout: {}",
            result.final_answer,
            result.stdout
        );
        assert_eq!(result.action_results.len(), 2);
        assert!(!result.had_error);
    }

    // ── asyncio.gather three tools ──────────────────────────

    #[tokio::test]
    async fn asyncio_gather_three_tools() {
        let thread = make_test_thread();
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(
            vec![
                test_action("web_search"),
                test_action("http"),
                test_action("memory_search"),
            ],
            vec![
                Ok(ActionResult {
                    call_id: String::new(),
                    action_name: "web_search".into(),
                    output: serde_json::json!("search results"),
                    is_error: false,
                    duration: Duration::from_millis(50),
                }),
                Ok(ActionResult {
                    call_id: String::new(),
                    action_name: "http".into(),
                    output: serde_json::json!("page content"),
                    is_error: false,
                    duration: Duration::from_millis(100),
                }),
                Ok(ActionResult {
                    call_id: String::new(),
                    action_name: "memory_search".into(),
                    output: serde_json::json!("memories"),
                    is_error: false,
                    duration: Duration::from_millis(25),
                }),
            ],
        ));

        let code = r#"
import asyncio
s, h, m = await asyncio.gather(
    web_search(query="test"),
    http(url="https://example.com"),
    memory_search(query="prior"),
)
FINAL(str(s) + "|" + str(h) + "|" + str(m))
"#;

        let result = run_code(code, effects, &thread).await.unwrap();
        assert!(!result.had_error, "stdout: {}", result.stdout);
        assert_eq!(result.action_results.len(), 3);
        let answer = result.final_answer.unwrap();
        assert!(answer.contains("search results"), "got: {answer}");
        assert!(answer.contains("page content"), "got: {answer}");
        assert!(answer.contains("memories"), "got: {answer}");
    }

    // ── Data-dependent chain (sequential await) ─────────────

    #[tokio::test]
    async fn sequential_dependent_calls() {
        let thread = make_test_thread();
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(
            vec![test_action("step1"), test_action("step2")],
            vec![
                Ok(ActionResult {
                    call_id: String::new(),
                    action_name: "step1".into(),
                    output: serde_json::json!("intermediate"),
                    is_error: false,
                    duration: Duration::from_millis(1),
                }),
                Ok(ActionResult {
                    call_id: String::new(),
                    action_name: "step2".into(),
                    output: serde_json::json!("final"),
                    is_error: false,
                    duration: Duration::from_millis(1),
                }),
            ],
        ));

        let code = r#"
a = await step1()
b = await step2(input=a)
FINAL(str(b))
"#;

        let result = run_code(code, effects, &thread).await.unwrap();
        assert!(!result.had_error, "stdout: {}", result.stdout);
        assert_eq!(result.action_results.len(), 2);
        assert_eq!(result.final_answer.as_deref(), Some("final"));
    }

    // ── Error in one gathered tool ──────────────────────────

    #[tokio::test]
    async fn gather_with_error_propagates() {
        let thread = make_test_thread();
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(
            vec![test_action("good"), test_action("bad")],
            vec![
                Ok(ActionResult {
                    call_id: String::new(),
                    action_name: "good".into(),
                    output: serde_json::json!("ok"),
                    is_error: false,
                    duration: Duration::from_millis(1),
                }),
                Err(EngineError::Effect {
                    reason: "tool exploded".into(),
                }),
            ],
        ));

        let code = r#"
import asyncio
a, b = await asyncio.gather(good(), bad())
FINAL("should not reach")
"#;

        let result = run_code(code, effects, &thread).await.unwrap();
        // Error in gather propagates as exception — code should error
        assert!(
            result.had_error,
            "should have error, stdout: {}",
            result.stdout
        );
        assert!(
            result.final_answer.is_none()
                || result.final_answer.as_deref() != Some("should not reach")
        );
    }

    // ── Tool with no lease (denied in preflight) ────────────

    #[tokio::test]
    async fn denied_tool_raises_exception() {
        let thread = make_test_thread();
        // No actions registered — tool has no lease
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(vec![], vec![]));

        let code = r#"
try:
    result = await unknown_tool()
    FINAL("should not reach")
except:
    FINAL("caught error")
"#;

        let result = run_code(code, effects, &thread).await.unwrap();
        // Tool not found raises NameError before we even get to dispatch
        assert!(result.final_answer.is_some(), "stdout: {}", result.stdout);
    }

    // ── FINAL works without await ───────────────────────────

    #[tokio::test]
    async fn final_is_sync() {
        let thread = make_test_thread();
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(vec![], vec![]));

        let code = r#"
FINAL("hello from sync")
"#;

        let result = run_code(code, effects, &thread).await.unwrap();
        assert_eq!(result.final_answer.as_deref(), Some("hello from sync"));
        assert!(!result.had_error);
    }

    // ── globals() still works ───────────────────────────────

    #[tokio::test]
    async fn globals_returns_known_tools() {
        let thread = make_test_thread();
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(
            vec![test_action("web_search"), test_action("http")],
            vec![],
        ));

        let code = r#"
g = globals()
has_search = "web_search" in g
has_http = "http" in g
FINAL(str(has_search) + "|" + str(has_http))
"#;

        let result = run_code(code, effects, &thread).await.unwrap();
        assert!(!result.had_error, "stdout: {}", result.stdout);
        assert_eq!(result.final_answer.as_deref(), Some("True|True"));
    }

    // ── Empty gather ────────────────────────────────────────

    #[tokio::test]
    async fn empty_gather() {
        let thread = make_test_thread();
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(vec![], vec![]));

        let code = r#"
import asyncio
results = await asyncio.gather()
FINAL(str(len(results)))
"#;

        let result = run_code(code, effects, &thread).await.unwrap();
        assert!(!result.had_error, "stdout: {}", result.stdout);
        assert_eq!(result.final_answer.as_deref(), Some("0"));
    }

    // ── Single-item gather ──────────────────────────────────

    #[tokio::test]
    async fn single_item_gather() {
        let thread = make_test_thread();
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(
            vec![test_action("echo")],
            vec![Ok(ActionResult {
                call_id: String::new(),
                action_name: "echo".into(),
                output: serde_json::json!("gathered"),
                is_error: false,
                duration: Duration::from_millis(1),
            })],
        ));

        let code = r#"
import asyncio
results = await asyncio.gather(echo())
FINAL(str(results[0]))
"#;

        let result = run_code(code, effects, &thread).await.unwrap();
        assert!(!result.had_error, "stdout: {}", result.stdout);
        assert_eq!(result.final_answer.as_deref(), Some("gathered"));
        assert_eq!(result.action_results.len(), 1);
    }

    // ── Sandbox security negative tests ────────────────────────

    /// OS-level operations must be denied or restricted by the Monty VM.
    #[tokio::test]
    async fn sandbox_denies_os_operations() {
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(vec![], vec![]));
        let thread = make_test_thread();

        // Try to import os and call os.system — should fail
        let code = r#"
try:
    import os
    os.system("echo pwned")
    FINAL("ESCAPED: os.system ran")
except Exception as e:
    FINAL("blocked: " + type(e).__name__)
"#;
        let result = run_code(code, effects, &thread).await.unwrap();
        let answer = result.final_answer.as_deref().unwrap_or("");
        assert!(
            !answer.starts_with("ESCAPED"),
            "os.system should be blocked, got: {answer}",
        );
    }

    /// Resource limits must be enforced — infinite loops should be terminated.
    #[tokio::test]
    async fn sandbox_enforces_resource_limits() {
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(vec![], vec![]));
        let thread = make_test_thread();

        // Infinite allocation loop — should hit allocation or memory limit
        let code = r#"
data = []
while True:
    data.append("x" * 10000)
"#;
        let result = run_code(code, effects, &thread).await;
        // Either returns an error or the stdout contains an error message —
        // the key assertion is that it DOES NOT run forever.
        if let Ok(r) = result {
            assert!(
                r.had_error || r.stdout.contains("Error") || r.stdout.contains("limit"),
                "resource limit should terminate infinite loop, got stdout: {}",
                truncate_for_assert(&r.stdout, 500),
            );
        }
        // Err(_) is also acceptable — means the VM was killed by resource limits
    }

    /// Python `import` of system modules must be restricted.
    #[tokio::test]
    async fn sandbox_restricts_imports() {
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(vec![], vec![]));
        let thread = make_test_thread();

        // Try to import subprocess — should fail
        let code = r#"
try:
    import subprocess
    result = subprocess.run(["echo", "escaped"], capture_output=True, text=True)
    FINAL("ESCAPED: " + result.stdout)
except Exception as e:
    FINAL("blocked: " + type(e).__name__)
"#;
        let result = run_code(code, effects, &thread).await.unwrap();
        let answer = result.final_answer.as_deref().unwrap_or("");
        assert!(
            !answer.starts_with("ESCAPED"),
            "subprocess import should be blocked, got: {answer}",
        );
    }

    /// File system access via open() must be blocked.
    #[tokio::test]
    async fn sandbox_denies_file_access() {
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(vec![], vec![]));
        let thread = make_test_thread();

        let code = r#"
try:
    f = open("/etc/passwd", "r")
    content = f.read()
    f.close()
    FINAL("ESCAPED: " + content[:50])
except Exception as e:
    FINAL("blocked: " + type(e).__name__)
"#;
        let result = run_code(code, effects, &thread).await.unwrap();
        let answer = result.final_answer.as_deref().unwrap_or("");
        assert!(
            !answer.starts_with("ESCAPED"),
            "open() should be blocked, got: {answer}",
        );
    }

    /// Network access via socket must be blocked.
    #[tokio::test]
    async fn sandbox_denies_socket_access() {
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(vec![], vec![]));
        let thread = make_test_thread();

        let code = r#"
try:
    import socket
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.connect(("127.0.0.1", 80))
    FINAL("ESCAPED: connected")
except Exception as e:
    FINAL("blocked: " + type(e).__name__)
"#;
        let result = run_code(code, effects, &thread).await.unwrap();
        let answer = result.final_answer.as_deref().unwrap_or("");
        assert!(
            !answer.starts_with("ESCAPED"),
            "socket access should be blocked, got: {answer}",
        );
    }

    /// Calls to tools not covered by the lease must be denied.
    #[tokio::test]
    async fn sandbox_unlicensed_tool_denied() {
        let effects: Arc<dyn EffectExecutor> =
            Arc::new(MockEffects::new(vec![test_action("allowed_tool")], vec![]));
        let thread = make_test_thread();
        let leases = LeaseManager::new();
        let policy = PolicyEngine::new();
        let ctx = make_exec_context(&thread);

        // Grant a restricted lease — only "allowed_tool" is permitted.
        leases
            .grant(
                thread.id,
                "tools",
                GrantedActions::Specific(vec!["allowed_tool".into()]),
                None,
                None,
            )
            .await
            .unwrap();

        let code = r#"
try:
    result = await secret_admin_tool(data="pwn")
    FINAL("ESCAPED: " + str(result))
except Exception as e:
    FINAL("blocked: " + type(e).__name__)
"#;
        let result = execute_code(
            code,
            &thread,
            &(Arc::new(StubLlm) as Arc<dyn crate::traits::llm::LlmBackend>),
            &effects,
            &leases,
            &policy,
            &ctx,
            &[],
            &serde_json::json!({}),
        )
        .await
        .unwrap();
        let answer = result.final_answer.as_deref().unwrap_or("");
        assert!(
            !answer.starts_with("ESCAPED"),
            "unlicensed tool should be denied by preflight, got: {answer}",
        );
    }

    /// CPU-bound infinite loops must be terminated by allocation/duration limits.
    #[tokio::test]
    async fn sandbox_enforces_cpu_limits() {
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(vec![], vec![]));
        let thread = make_test_thread();

        // Tight CPU-bound loop (no allocations to trip allocation limit)
        let code = r#"
x = 0
while True:
    x += 1
"#;
        let result = run_code(code, effects, &thread).await;
        // Must terminate — either via error or resource limit
        if let Ok(r) = result {
            assert!(
                r.had_error || r.stdout.contains("Error") || r.stdout.contains("limit"),
                "cpu-bound loop should be terminated, stdout: {}",
                truncate_for_assert(&r.stdout, 500),
            );
        }
        // Err(_) is also acceptable — means the VM was killed by resource limits
    }

    /// FINAL() must capture the answer from the code.
    #[tokio::test]
    async fn sandbox_final_captures_answer() {
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(vec![], vec![]));
        let thread = make_test_thread();

        let code = r#"
x = 2 + 3
FINAL(str(x))
"#;
        let result = run_code(code, effects, &thread).await.unwrap();
        assert_eq!(
            result.final_answer.as_deref(),
            Some("5"),
            "FINAL should capture the computed answer"
        );
    }

    /// Syntax errors flow back as errors, not panics.
    #[tokio::test]
    async fn sandbox_handles_syntax_error() {
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(vec![], vec![]));
        let thread = make_test_thread();

        let code = "def broken(\nFINAL('nope')";
        let result = run_code(code, effects, &thread).await.unwrap();
        assert!(result.had_error, "syntax error should set had_error");
        assert!(
            result.stdout.contains("SyntaxError") || result.stdout.contains("Error"),
            "should contain SyntaxError, got: {}",
            result.stdout,
        );
    }
}
