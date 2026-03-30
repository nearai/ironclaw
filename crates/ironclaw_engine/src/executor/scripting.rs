//! Tier 1 executor: embedded Python via Monty.
//!
//! Executes LLM-generated Python code using the Monty interpreter. Tool
//! calls happen as regular function calls in the code — Monty suspends at
//! each unknown function, and we delegate to the `EffectExecutor`.
//!
//! Follows the RLM (Recursive Language Model) pattern:
//! - Thread context injected as Python variables (not LLM attention input)
//! - `llm_query()` / `llm_query_batched()` for recursive subagent spawning
//! - `FINAL(answer)` / `FINAL_VAR(name)` for explicit termination
//! - Step 0 orientation preamble for context awareness
//! - Errors flow back to LLM for self-correction (not step termination)
//! - Output truncated to configurable limit with variable listing

use std::sync::Arc;
use std::time::Duration;

use monty::{
    ExcType, ExtFunctionResult, LimitedTracker, MontyException, MontyObject, MontyRun,
    NameLookupResult, PrintWriter, ResourceLimits, RunProgress,
};
use tracing::{debug, warn};

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
        if stdout.chars().count() > OUTPUT_TRUNCATE_LEN {
            let truncated: String = stdout
                .chars()
                .skip(stdout.chars().count() - OUTPUT_TRUNCATE_LEN)
                .collect();
            parts.push(format!(
                "[TRUNCATED: last {OUTPUT_TRUNCATE_LEN} of {} chars shown]\n{truncated}",
                stdout.len()
            ));
        } else {
            parts.push(format!("[FULL OUTPUT: {} chars]\n{stdout}", stdout.len()));
        }
    }

    if *return_value != serde_json::Value::Null {
        let val_str = serde_json::to_string_pretty(return_value).unwrap_or_default();
        if val_str.len() > OUTPUT_PREVIEW_LEN {
            let preview: String = val_str.chars().take(OUTPUT_PREVIEW_LEN).collect();
            parts.push(format!(
                "Return value ({} chars): {preview}...",
                val_str.len()
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
                let call_id = format!("code_call_{call_counter}");
                let action_name = call.function_name.clone();
                let params = monty_args_to_json(&call.args, &call.kwargs);

                debug!(action = %action_name, call_id = %call_id, "Monty: function call");

                let ext_result = match action_name.as_str() {
                    // FINAL(answer) — explicit termination
                    "FINAL" => {
                        let answer = call.args.first().map(monty_to_string).unwrap_or_default();
                        final_answer = Some(answer);
                        ExtFunctionResult::Return(MontyObject::None)
                    }

                    // FINAL_VAR(name) — terminate with variable value
                    // (the variable's value is whatever the code stored in it;
                    // we return None and the complete handler reads final_answer)
                    "FINAL_VAR" => {
                        let var_name = call
                            .args
                            .first()
                            .map(monty_to_string)
                            .unwrap_or_else(|| "result".into());
                        // We can't access the REPL's namespace directly from here,
                        // so we store the variable name and let the caller handle it.
                        // For now, FINAL_VAR works the same as FINAL with the var name.
                        final_answer = Some(format!("[FINAL_VAR: {var_name}]"));
                        ExtFunctionResult::Return(MontyObject::None)
                    }

                    // llm_query(prompt, context) — recursive sub-call
                    "llm_query" => {
                        handle_llm_query(&call.args, &call.kwargs, llm, &mut recursive_tokens).await
                    }

                    // llm_query_batched(prompts) — parallel sub-calls
                    "llm_query_batched" => {
                        handle_llm_query_batched(
                            &call.args,
                            &call.kwargs,
                            llm,
                            &mut recursive_tokens,
                        )
                        .await
                    }

                    // rlm_query(prompt) — full recursive sub-agent with own CodeAct loop
                    "rlm_query" => {
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
                        .await
                    }

                    // globals() / locals() — return dict with known names so
                    // `"tool_name" in globals()` works for capability probing
                    "globals" | "locals" => {
                        let entries: Vec<(MontyObject, MontyObject)> = known_actions
                            .iter()
                            .map(|name| {
                                (MontyObject::String(name.clone()), MontyObject::Bool(true))
                            })
                            .collect();
                        ExtFunctionResult::Return(MontyObject::Dict(entries.into()))
                    }

                    // Regular tool dispatch
                    _ => {
                        let dispatch = dispatch_action(
                            &action_name,
                            &call_id,
                            params.clone(),
                            thread,
                            effects,
                            leases,
                            policy,
                            context,
                            capability_policies,
                            &mut action_results,
                            &mut events,
                        )
                        .await;

                        match dispatch {
                            DispatchResult::Ok(r) => r,
                            DispatchResult::NeedApproval => {
                                return Ok(CodeExecutionResult {
                                    return_value: serde_json::Value::Null,
                                    stdout,
                                    action_results,
                                    events,
                                    need_approval: Some(
                                        crate::runtime::messaging::ThreadOutcome::NeedApproval {
                                            action_name,
                                            call_id,
                                            parameters: params,
                                        },
                                    ),
                                    recursive_tokens,
                                    final_answer: None,
                                    had_error,
                                });
                            }
                            DispatchResult::NeedAuthentication {
                                credential_name,
                                action_name,
                                call_id,
                                parameters,
                            } => {
                                return Ok(CodeExecutionResult {
                                    return_value: serde_json::Value::Null,
                                    stdout,
                                    action_results,
                                    events,
                                    need_approval: Some(
                                        crate::runtime::messaging::ThreadOutcome::NeedAuthentication {
                                            credential_name,
                                            action_name,
                                            call_id,
                                            parameters,
                                        },
                                    ),
                                    recursive_tokens,
                                    final_answer: None,
                                    had_error,
                                });
                            }
                        }
                    }
                };

                // Resume Monty (with error recovery — don't terminate on Monty errors)
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    call.resume(ext_result, PrintWriter::Collect(&mut stdout))
                })) {
                    Ok(Ok(p)) => progress = p,
                    Ok(Err(e)) => {
                        // Runtime error after resume → include in output, mark as error
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

            RunProgress::NameLookup(lookup) => {
                let name = lookup.name.clone();

                // If the name matches a known tool, return a callable Function
                // stub so Monty yields FunctionCall (dispatched to the effect
                // executor) instead of raising NameError.
                let result = if known_actions.contains(&name) {
                    debug!(name = %name, "Monty: resolved as tool function");
                    NameLookupResult::Value(MontyObject::Function {
                        name: name.clone(),
                        docstring: None,
                    })
                } else if name == "globals" || name == "locals" {
                    // Python builtins for namespace introspection — resolve as
                    // callable so code like `"tool" in globals()` works.
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
                warn!(function = ?os_call.function, "Monty: OS call denied");
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

            RunProgress::ResolveFutures(_) => {
                // Async not supported — return error to LLM
                stdout.push_str("\nError: async/await is not supported in CodeAct scripts");
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
        }
    }
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
        let lease = child_leases
            .grant(
                child_thread.id,
                &parent_lease.capability_name,
                parent_lease.granted_actions.clone(),
                remaining_duration,
                parent_lease.max_uses,
            )
            .await;
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

// ── Dispatch ────────────────────────────────────────────────

enum DispatchResult {
    Ok(ExtFunctionResult),
    NeedApproval,
    NeedAuthentication {
        credential_name: String,
        action_name: String,
        call_id: String,
        parameters: serde_json::Value,
    },
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_action(
    action_name: &str,
    call_id: &str,
    params: serde_json::Value,
    thread: &Thread,
    effects: &Arc<dyn EffectExecutor>,
    leases: &LeaseManager,
    policy: &PolicyEngine,
    context: &ThreadExecutionContext,
    capability_policies: &[crate::types::capability::PolicyRule],
    action_results: &mut Vec<ActionResult>,
    events: &mut Vec<EventKind>,
) -> DispatchResult {
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
            return DispatchResult::Ok(ExtFunctionResult::NotFound(action_name.into()));
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
                return DispatchResult::Ok(ExtFunctionResult::Error(MontyException::new(
                    ExcType::RuntimeError,
                    Some(format!("denied: {reason}")),
                )));
            }
            PolicyDecision::RequireApproval { .. } => {
                events.push(EventKind::ApprovalRequested {
                    action_name: action_name.into(),
                    call_id: call_id.into(),
                });
                return DispatchResult::NeedApproval;
            }
            PolicyDecision::Allow => {}
        }
    }

    if let Err(e) = leases.consume_use(lease.id).await {
        return DispatchResult::Ok(ExtFunctionResult::Error(MontyException::new(
            ExcType::RuntimeError,
            Some(format!("lease exhausted: {e}")),
        )));
    }

    let ps = crate::types::event::summarize_params(action_name, &params);
    let params_for_auth = params.clone();

    match effects
        .execute_action(action_name, params, &lease, context)
        .await
    {
        Ok(result) => {
            events.push(EventKind::ActionExecuted {
                step_id: context.step_id,
                action_name: action_name.into(),
                call_id: call_id.into(),
                duration_ms: result.duration.as_millis() as u64,
                params_summary: ps,
            });
            let monty_obj = json_to_monty(&result.output);
            action_results.push(result);
            DispatchResult::Ok(ExtFunctionResult::Return(monty_obj))
        }
        Err(EngineError::NeedApproval { .. }) => {
            events.push(EventKind::ApprovalRequested {
                action_name: action_name.into(),
                call_id: call_id.into(),
            });
            DispatchResult::NeedApproval
        }
        Err(EngineError::NeedAuthentication {
            credential_name, ..
        }) => {
            events.push(EventKind::ActionFailed {
                step_id: context.step_id,
                action_name: action_name.into(),
                call_id: call_id.into(),
                error: format!("authentication required for credential '{credential_name}'"),
                params_summary: ps,
            });
            DispatchResult::NeedAuthentication {
                credential_name,
                action_name: action_name.into(),
                call_id: call_id.into(),
                parameters: params_for_auth,
            }
        }
        Err(e) => {
            action_results.push(ActionResult {
                call_id: call_id.into(),
                action_name: action_name.into(),
                output: serde_json::json!({"error": e.to_string()}),
                is_error: true,
                duration: Duration::ZERO,
            });
            events.push(EventKind::ActionFailed {
                step_id: context.step_id,
                action_name: action_name.into(),
                call_id: call_id.into(),
                error: e.to_string(),
                params_summary: ps,
            });
            DispatchResult::Ok(ExtFunctionResult::Error(MontyException::new(
                ExcType::RuntimeError,
                Some(e.to_string()),
            )))
        }
    }
}

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
