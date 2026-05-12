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

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

tokio::task_local! {
    /// Side-channel between `drive_inline_gate`'s `Cancelled+Authentication`
    /// fallback and `execute_code`'s exit. When the inline-await for an
    /// Authentication gate cancels (e.g. because the controller has no
    /// `PerExecutionContext` registered — the typical case for mission
    /// child threads), the fallback writes the original `ThreadOutcome::GatePaused`
    /// here before raising the legacy `RuntimeError("execution paused by
    /// gate ...")`. `execute_code` reads it on the way out and surfaces
    /// it as `CodeExecutionResult::need_approval`, which the orchestrator
    /// then converts to `ThreadOutcome::GatePaused` so the mission flow
    /// (#3133 half-1) transitions the mission to Paused.
    ///
    /// Without this, Tier 1 mission child threads would silently swallow
    /// the gate, the mission would stay Active, and the cron would keep
    /// re-firing — the original #3133 ghost-fire pattern.
    static PENDING_GATE_STASH:
        RefCell<Option<crate::runtime::messaging::ThreadOutcome>>;
}

/// Drain the per-execution `PENDING_GATE_STASH`. Called at every
/// script-error exit in `execute_code_with_skills_inner` so that a
/// `Cancelled+Authentication` gate raised during a Monty `call.resume`
/// surfaces as `CodeExecutionResult::need_approval` regardless of which
/// resume path the script error propagates through. Returns `None` when
/// the task-local isn't in scope (only happens outside `execute_code`).
fn take_pending_gate_stash() -> Option<crate::runtime::messaging::ThreadOutcome> {
    PENDING_GATE_STASH
        .try_with(|cell| cell.borrow_mut().take())
        .ok()
        .flatten()
}

use monty::{
    ExcType, ExtFunctionResult, LimitedTracker, MontyDate, MontyDateTime, MontyException,
    MontyObject, MontyRun, NameLookupResult, OsFunction, PrintWriter, ResourceLimits, RunProgress,
};
use tracing::debug;

use crate::capability::lease::LeaseManager;
use crate::capability::policy::{PolicyDecision, PolicyEngine};
use crate::traits::effect::{EffectExecutor, ThreadExecutionContext};
use crate::traits::llm::{LlmBackend, LlmCallConfig};
use crate::types::capability::ActionDef;
use crate::types::error::EngineError;
use crate::types::event::EventKind;
use crate::types::message::{MessageRole, ThreadMessage};
use crate::types::step::{ActionResult, CodeExecutionFailure, LlmResponse, TokenUsage};
use crate::types::thread::Thread;
use ironclaw_common::ValidTimezone;

// ── Configuration ───────────────────────────────────────────

/// Maximum characters of output to include in LLM context between steps.
/// Matches Prime Intellect's default. Configurable per thread in the future.
const OUTPUT_TRUNCATE_LEN: usize = 8_000;

/// Maximum characters for a preview prefix in compact metadata.
const OUTPUT_PREVIEW_LEN: usize = 200;

/// Build a `MontyObject::DateTime` for the current instant.
///
/// Honors `args[0]` when it is a `MontyTimeZone` (aware datetime with that
/// fixed offset) or `MontyObject::None` (naive datetime in UTC, matching
/// CPython's `datetime.datetime.now()` behavior without a tz). Anything
/// else is treated as "no tz" rather than raising — we prefer the LLM get
/// a usable clock read even if it passes a weird argument.
fn build_datetime_now(args: &[MontyObject]) -> MontyObject {
    use chrono::{DateTime, Datelike, FixedOffset, Timelike, Utc};

    let utc_now: DateTime<Utc> = Utc::now();

    let (offset_seconds, timezone_name) = match args.first() {
        Some(MontyObject::TimeZone(tz)) => (Some(tz.offset_seconds), tz.name.clone()),
        _ => (None, None),
    };

    let aware = offset_seconds
        .and_then(FixedOffset::east_opt)
        .map(|offset| utc_now.with_timezone(&offset));

    let (year, month, day, hour, minute, second, microsecond) = if let Some(dt) = aware {
        (
            dt.year(),
            dt.month() as u8,
            dt.day() as u8,
            dt.hour() as u8,
            dt.minute() as u8,
            dt.second() as u8,
            dt.timestamp_subsec_micros(),
        )
    } else {
        (
            utc_now.year(),
            utc_now.month() as u8,
            utc_now.day() as u8,
            utc_now.hour() as u8,
            utc_now.minute() as u8,
            utc_now.second() as u8,
            utc_now.timestamp_subsec_micros(),
        )
    };

    MontyObject::DateTime(MontyDateTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
        microsecond,
        offset_seconds,
        timezone_name,
    })
}

/// Build a `MontyObject::Date` for today's UTC date.
///
/// Python's `date.today()` is timezone-naive (local date on CPython); we
/// return UTC to avoid host-clock timezone surprises inside the sandbox.
/// Agents that need a local date should call the `time` tool with an
/// explicit timezone.
fn build_date_today() -> MontyObject {
    use chrono::{Datelike, Utc};

    let today = Utc::now().date_naive();
    MontyObject::Date(MontyDate {
        year: today.year(),
        month: today.month() as u8,
        day: today.day() as u8,
    })
}

/// Default resource limits for Monty execution.
///
/// `max_duration` is wall-clock from VM start and ticks during inline
/// gate-await pauses (we await user input *inside* the same Monty
/// execution). 30s is what catches runaway CPU-bound scripts that
/// don't allocate (`while True: x += 1`); raising it to "30 min so
/// human approvals fit" hangs those tests. Tradeoff: with 30s, an
/// approval that takes longer than 30s timeouts the script and the
/// user has to retry. Most approvals come back in seconds; longer
/// ones are a documented limitation. A proper "active CPU vs paused"
/// timer split is on the follow-up list (see
/// `docs/plans/2026-05-01-codeact-inline-gate-await.md`).
fn default_limits() -> ResourceLimits {
    ResourceLimits::new()
        .max_duration(Duration::from_secs(30))
        .max_allocations(1_000_000)
        .max_memory(64 * 1024 * 1024) // 64 MB
}

// ── Validation ─────────────────────────────────────────────

/// Maximum orchestrator source size accepted for syntax validation (256 KB).
/// The compiled-in default is ~2 KB; this cap is generous but prevents
/// pathological inputs from causing avoidable CPU/memory pressure on the
/// store write path.
const MAX_ORCHESTRATOR_SOURCE_BYTES: usize = 256 * 1024;

/// Check whether `code` is syntactically valid Python without executing it.
///
/// Uses Monty's parser (same as execution) so the syntax check is identical
/// to what would happen at runtime. Returns `Ok(())` if valid, or an error
/// message describing the syntax problem.
///
/// **Threat model**: syntax validation prevents broken patches from consuming
/// failure-budget slots (3 consecutive failures trigger auto-rollback), NOT
/// from executing dangerous code. Semantically dangerous patterns
/// (`exec(compile(...))`, `__import__('os')`) pass validation because they
/// are syntactically valid Python. All security enforcement happens at
/// runtime in the Monty sandbox (resource limits, host-function gating, no
/// filesystem/network access).
///
/// **Runtime cost**: `MontyRun::new()` **parses and prepares only** — it
/// builds the AST and interns, but does not allocate the heap, create
/// namespaces, or step any Python instructions. Upstream docstring:
/// "This only parses and prepares the code - no heap or namespaces are
/// created yet. Call `run_snapshot()` with inputs to start execution."
/// No module-level code runs here. Cost scales with parser input size,
/// so we bound inputs at `MAX_ORCHESTRATOR_SOURCE_BYTES` (256 KB; the
/// compiled-in default is ~2 KB) to keep the store write path from
/// becoming a CPU/memory amplifier for pathological patches. The call is
/// wrapped in `catch_unwind` because the Monty parser, like most
/// hand-written Rust parsers, is not panic-audited for every adversarial
/// input.
pub fn validate_python_syntax(code: &str) -> Result<(), String> {
    if code.len() > MAX_ORCHESTRATOR_SOURCE_BYTES {
        return Err(format!(
            "orchestrator source too large: {} bytes (limit: {MAX_ORCHESTRATOR_SOURCE_BYTES})",
            code.len()
        ));
    }
    let code_owned = code.to_string();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        MontyRun::new(code_owned, "validate.py", vec![])
    })) {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(format!("syntax error: {e}")),
        Err(_) => Err("parser panic during syntax validation".into()),
    }
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
    /// Classified failure category. `None` when execution succeeded or was
    /// paused by a gate. `Some(category)` when code execution failed —
    /// `failure.is_some()` replaces the former `had_error: bool` field.
    pub failure: Option<CodeExecutionFailure>,
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

// ── Gate resolution mapping ─────────────────────────────────

/// Why a gate did not approve. Distinguishes user-driven denial from
/// "no live approval handler reached the user" so script-facing and
/// event-log messages don't mislabel a cancellation/expiry as a user
/// denial.
///
/// Wrapping behavior used to be `format!("user denied tool 'X': {reason}")`
/// for every non-`Approved` resolution; that incorrectly read
/// "user denied tool 'X': cancelled" when the script ran under
/// [`crate::gate::CancellingGateController`] (no controller wired) or
/// when the bridge controller cancelled on expiry/shutdown. The user
/// never saw a prompt in those cases — they didn't deny anything.
///
/// Helpers here produce the right wording per surface
/// (event-log error, script-facing exception, `EngineError::Effect`
/// reason) so all Tier 0 / Tier 1 call sites stay consistent.
#[derive(Debug, Clone)]
pub(crate) enum DenialOutcome {
    /// User actively denied the gate (or the host's controller treats
    /// "no input" as deny). Reason text typically comes from the user
    /// or the controller's deny reason.
    DeniedByUser { reason: String },
    /// No live approval handler reached the user — controller missing
    /// (`CancellingGateController`), bridge controller cancelled on
    /// expiry/shutdown, or the engine got back a resolution variant
    /// the inline path doesn't support.
    Unavailable { detail: String },
}

impl DenialOutcome {
    /// Pre-formatted `error` string for `EventKind::ActionFailed`.
    /// Surfaces in trace/audit/observer paths; a "denied:" prefix here
    /// lined up with the policy-deny path before the gate controller
    /// existed, so user-driven denials keep that prefix for continuity.
    /// `Unavailable` uses a distinct prefix so an operator scanning
    /// logs can tell apart "user said no" from "no prompt was shown".
    pub(crate) fn event_error(&self) -> String {
        match self {
            Self::DeniedByUser { reason } => format!("denied: {reason}"),
            Self::Unavailable { detail } => format!("approval unavailable: {detail}"),
        }
    }

    /// Pre-formatted `RuntimeError` message for CodeAct scripts.
    /// Identifies the tool by name so scripts can branch on the
    /// failure cause, and surfaces the distinction between
    /// user-driven denial and no-handler/cancelled directly in the
    /// message text — pre-fix the latter incorrectly read
    /// "user denied tool 'X': cancelled".
    pub(crate) fn script_message(&self, tool_name: &str) -> String {
        match self {
            Self::DeniedByUser { reason } => {
                format!("user denied tool '{tool_name}': {reason}")
            }
            Self::Unavailable { detail } => {
                format!("approval for tool '{tool_name}' unavailable: {detail}")
            }
        }
    }

    /// Bare reason string for `EngineError::Effect` (Tier 0 structured
    /// path, where the error gets bubbled up rather than rendered as
    /// a Python exception). Same shape as `event_error`.
    pub(crate) fn effect_reason(&self) -> String {
        self.event_error()
    }
}

/// Single source of truth shared by Tier 0 (`structured.rs`) and Tier 1
/// (sync preflight + async output paths in this module) so denial
/// messages can't drift between executors.
///
/// Returns `None` for `Approved` (the only outcome that lets execution
/// continue).
pub(crate) fn denial_outcome_for_resolution(
    resolution: &crate::gate::GateResolution,
) -> Option<DenialOutcome> {
    match resolution {
        crate::gate::GateResolution::Approved { .. } => None,
        crate::gate::GateResolution::Denied { reason } => Some(DenialOutcome::DeniedByUser {
            reason: reason.clone().unwrap_or_else(|| "denied by user".into()),
        }),
        crate::gate::GateResolution::Cancelled => Some(DenialOutcome::Unavailable {
            detail: "approval cancelled".into(),
        }),
        crate::gate::GateResolution::CredentialProvided { .. }
        | crate::gate::GateResolution::ExternalCallback { .. } => {
            Some(DenialOutcome::Unavailable {
                detail: "unsupported gate resolution".into(),
            })
        }
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

    // `user_timezone` — validated IANA timezone from the user's channel (e.g. "America/New_York")
    let tz = thread
        .metadata
        .get("user_timezone")
        .and_then(|v| v.as_str())
        .and_then(ValidTimezone::parse)
        .map(|vtz| vtz.name().to_string())
        .unwrap_or_else(|| "UTC".into());
    names.push("user_timezone".into());
    values.push(MontyObject::String(tz));

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
    // Scope the per-execution PENDING_GATE_STASH task-local so the
    // inline-await Cancelled+Authentication fallback (deeper inside
    // `drive_inline_gate`) has a side channel to surface the original
    // gate as `need_approval` on the way out. See the static's
    // doc-comment for why this exists.
    PENDING_GATE_STASH
        .scope(RefCell::new(None), async move {
            execute_code_with_skills_inner(
                code,
                thread,
                llm,
                effects,
                leases,
                policy,
                context,
                capability_policies,
                persisted_state,
                skill_snippet_names,
            )
            .await
        })
        .await
}

#[allow(clippy::too_many_arguments)]
async fn execute_code_with_skills_inner(
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

    // Build context variables including persisted state from prior steps
    let (input_names, input_values) = build_context_inputs(thread, persisted_state);

    // Collect known tool names so NameLookup can return callable stubs.
    // Without this, `mission_list()` in code raises NameError because Monty
    // resolves the name before calling it, and Undefined → NameError.
    let active_leases = leases.active_for_thread(thread.id).await;
    let inventory = match effects
        .available_action_inventory(&active_leases, context)
        .await
    {
        Ok(inventory) => Some(Arc::new(inventory)),
        Err(error) => {
            debug!(
                thread_id = %thread.id,
                "failed to load action inventory for scripting execution: {error}"
            );
            None
        }
    };
    let available_actions: Arc<[ActionDef]> = inventory
        .as_ref()
        .map(|inventory| inventory.inline.clone().into())
        .unwrap_or_else(|| Arc::from([]));
    let mut execution_context = context.clone();
    if let Some(ref inventory) = inventory {
        execution_context.available_actions_snapshot = Some(Arc::clone(&available_actions));
        execution_context.available_action_inventory_snapshot = Some(Arc::clone(inventory));
    }
    let mut known_actions: std::collections::HashSet<String> = available_actions
        .iter()
        .map(|action| action.name.clone())
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
                failure: Some(CodeExecutionFailure::SyntaxError),
            });
        }
        Err(_) => {
            return Ok(CodeExecutionResult {
                return_value: serde_json::Value::Null,
                stdout: format!("{stdout}\nVmPanic: Monty VM panicked during code parsing"),
                action_results,
                events,
                need_approval: None,
                recursive_tokens,
                final_answer: None,
                failure: Some(CodeExecutionFailure::VmPanic),
            });
        }
    };

    // Start execution with resource limits and context inputs
    let tracker = LimitedTracker::new(default_limits());

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
            // Runtime error flows back to LLM. Before classifying it,
            // check the inline-await fallback stash: if a Cancelled+
            // Authentication gate fired and surfaced as a
            // `RuntimeError("execution paused by gate ...")`, surface
            // it as `need_approval` so the orchestrator can produce
            // `ThreadOutcome::GatePaused` and mission flows transition
            // to Paused. Without this, Tier 1 mission threads silently
            // swallow the gate (the original #3133 ghost-fire shape).
            let pending_gate = take_pending_gate_stash();
            let category = classify_runtime_error(&e.to_string());
            return Ok(CodeExecutionResult {
                return_value: serde_json::Value::Null,
                stdout: format!("{stdout}\nError: {e}"),
                action_results,
                events,
                need_approval: pending_gate,
                recursive_tokens,
                final_answer: None,
                failure: Some(category),
            });
        }
        Err(_) => {
            return Ok(CodeExecutionResult {
                return_value: serde_json::Value::Null,
                stdout: format!("{stdout}\nVmPanic: Monty VM panicked during execution start"),
                action_results,
                events,
                need_approval: None,
                recursive_tokens,
                final_answer: None,
                failure: Some(CodeExecutionFailure::VmPanic),
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
                    failure: None,
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
                //
                // FINAL / FINAL_VAR set `final_answer` synchronously but also
                // install a trivially-resolving pending future. That way both
                // `FINAL(x)` and `await FINAL(x)` are valid: the sync call
                // just discards the coroutine object, while `await` resolves
                // it to None. LLMs frequently emit `await FINAL(...)` by
                // analogy with tool calls, so supporting both avoids a whole
                // class of "NoneType can't be awaited" failures.
                let sync_result = match action_name.as_str() {
                    "FINAL" => {
                        let answer = call.args.first().map(monty_to_string).unwrap_or_default();
                        final_answer = Some(answer);
                        pending_futures.insert(monty_call_id, PendingFuture::ready_none());
                        None
                    }
                    "FINAL_VAR" => {
                        let var_name = call
                            .args
                            .first()
                            .map(monty_to_string)
                            .unwrap_or_else(|| "result".into());
                        final_answer = Some(format!("[FINAL_VAR: {var_name}]"));
                        pending_futures.insert(monty_call_id, PendingFuture::ready_none());
                        None
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
                            &execution_context.gate_controller,
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
                        call.resume(ext_result, PrintWriter::CollectString(&mut stdout))
                    })) {
                        Ok(Ok(p)) => progress = p,
                        Ok(Err(e)) => {
                            // Read the inline-await stash before the
                            // runtime-error exit. See `take_pending_gate_stash`.
                            let pending_gate = take_pending_gate_stash();
                            stdout.push_str(&format!("\nError: {e}"));
                            return Ok(CodeExecutionResult {
                                return_value: serde_json::Value::Null,
                                stdout,
                                action_results,
                                events,
                                need_approval: pending_gate,
                                recursive_tokens,
                                final_answer,
                                failure: Some(classify_runtime_error(&e.to_string())),
                            });
                        }
                        Err(_) => {
                            return Ok(CodeExecutionResult {
                                return_value: serde_json::Value::Null,
                                stdout: format!(
                                    "{stdout}\nVmPanic: Monty VM panicked during resume"
                                ),
                                action_results,
                                events,
                                need_approval: None,
                                recursive_tokens,
                                final_answer,
                                failure: Some(CodeExecutionFailure::VmPanic),
                            });
                        }
                    }
                    continue;
                }

                // If an LLM call already inserted a pending future, just
                // resume_pending and continue — no preflight needed.
                if pending_futures.contains_key(&monty_call_id) {
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        call.resume_pending(PrintWriter::CollectString(&mut stdout))
                    })) {
                        Ok(Ok(p)) => progress = p,
                        Ok(Err(e)) => {
                            let pending_gate = take_pending_gate_stash();
                            stdout.push_str(&format!("\nError: {e}"));
                            return Ok(CodeExecutionResult {
                                return_value: serde_json::Value::Null,
                                stdout,
                                action_results,
                                events,
                                need_approval: pending_gate,
                                recursive_tokens,
                                final_answer,
                                failure: Some(classify_runtime_error(&e.to_string())),
                            });
                        }
                        Err(_) => {
                            return Ok(CodeExecutionResult {
                                return_value: serde_json::Value::Null,
                                stdout: format!(
                                    "{stdout}\nVmPanic: Monty VM panicked during resume_pending"
                                ),
                                action_results,
                                events,
                                need_approval: None,
                                recursive_tokens,
                                final_answer,
                                failure: Some(CodeExecutionFailure::VmPanic),
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
                    leases,
                    policy,
                    &execution_context,
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
                        let mut ctx = execution_context.clone();
                        ctx.current_call_id = Some(str_call_id.clone());
                        let ps = crate::types::event::summarize_params(&name, &params);

                        let handle = tokio::spawn(async move {
                            let execution_start = Instant::now();
                            let result = effects
                                .execute_action(&name, params_clone, &lease_clone, &ctx)
                                .await;
                            (result, execution_start.elapsed().as_millis() as u64)
                        });

                        pending_futures.insert(
                            monty_call_id,
                            PendingFuture::Tool {
                                handle,
                                action_name,
                                call_id: str_call_id,
                                lease_id: lease.id,
                                params_summary: ps,
                            },
                        );

                        // Resume with pending future — Python gets ExternalFuture
                        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            call.resume_pending(PrintWriter::CollectString(&mut stdout))
                        })) {
                            Ok(Ok(p)) => progress = p,
                            Ok(Err(e)) => {
                                stdout.push_str(&format!("\nError: {e}"));
                                return Ok(CodeExecutionResult {
                                    return_value: serde_json::Value::Null,
                                    stdout,
                                    action_results,
                                    events,
                                    need_approval: None,
                                    recursive_tokens,
                                    final_answer,
                                    failure: Some(CodeExecutionFailure::ToolError),
                                });
                            }
                            Err(_) => {
                                return Ok(CodeExecutionResult {
                                    return_value: serde_json::Value::Null,
                                    stdout: format!(
                                        "{stdout}\nVmPanic: Monty VM panicked during resume_pending"
                                    ),
                                    action_results,
                                    events,
                                    need_approval: None,
                                    recursive_tokens,
                                    final_answer,
                                    failure: Some(CodeExecutionFailure::VmPanic),
                                });
                            }
                        }
                    }
                    PreflightResult::Denied(ext_result) => {
                        // Resume with error — Python sees an exception
                        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            call.resume(ext_result, PrintWriter::CollectString(&mut stdout))
                        })) {
                            Ok(Ok(p)) => progress = p,
                            Ok(Err(e)) => {
                                stdout.push_str(&format!("\nError: {e}"));
                                return Ok(CodeExecutionResult {
                                    return_value: serde_json::Value::Null,
                                    stdout,
                                    action_results,
                                    events,
                                    need_approval: None,
                                    recursive_tokens,
                                    final_answer,
                                    failure: Some(CodeExecutionFailure::ToolError),
                                });
                            }
                            Err(_) => {
                                return Ok(CodeExecutionResult {
                                    return_value: serde_json::Value::Null,
                                    stdout: format!(
                                        "{stdout}\nVmPanic: Monty VM panicked during resume"
                                    ),
                                    action_results,
                                    events,
                                    need_approval: None,
                                    recursive_tokens,
                                    final_answer,
                                    failure: Some(CodeExecutionFailure::VmPanic),
                                });
                            }
                        }
                    }
                    PreflightResult::GatePaused(outcome) => {
                        // Inline gate-await: keep the Monty VM alive,
                        // pause for the user, and continue from the
                        // exact suspension point on resolution. The
                        // controller is required on the context — code
                        // paths that don't pause supply
                        // `CancellingGateController`, which surfaces
                        // gates as a typed denial here.
                        let crate::runtime::messaging::ThreadOutcome::GatePaused {
                            gate_name,
                            action_name: gate_action_name,
                            call_id: gate_call_id,
                            parameters: gate_parameters,
                            resume_kind,
                            ..
                        } = outcome
                        else {
                            // ThreadOutcome::GatePaused is the only variant
                            // PreflightResult::GatePaused builds; falling
                            // back here would indicate a programmer error.
                            return Ok(CodeExecutionResult {
                                return_value: serde_json::Value::Null,
                                stdout,
                                action_results,
                                events,
                                need_approval: None,
                                recursive_tokens,
                                final_answer,
                                failure: Some(CodeExecutionFailure::ToolError),
                            });
                        };

                        let resolution = execution_context
                            .gate_controller
                            .pause(crate::gate::GatePauseRequest {
                                thread_id: thread.id,
                                user_id: thread.user_id.clone(),
                                gate_name: gate_name.clone(),
                                action_name: gate_action_name.clone(),
                                call_id: gate_call_id.clone(),
                                parameters: gate_parameters.clone(),
                                resume_kind: resume_kind.clone(),
                                conversation_id: execution_context.conversation_id,
                            })
                            .await;

                        let denial = denial_outcome_for_resolution(&resolution);

                        if let Some(outcome) = denial {
                            // Record the denial in the thread event log
                            // before resuming Monty so observers / trace
                            // analysis see consistent ActionFailed output
                            // across all denial paths (this site +
                            // `drive_inline_gate` + `structured.rs`).
                            events.push(EventKind::ActionFailed {
                                step_id: execution_context.step_id,
                                action_name: gate_action_name.clone(),
                                call_id: gate_call_id.clone(),
                                error: outcome.event_error(),
                                duration_ms: 0,
                                params_summary: crate::types::event::summarize_params(
                                    &gate_action_name,
                                    &gate_parameters,
                                ),
                            });
                            // Resume Monty with a typed exception. RuntimeError
                            // is what we emit; the message is explicit so users
                            // (and the LLM) can distinguish denial from other
                            // runtime errors. `script_message` distinguishes a
                            // user-driven denial ("user denied tool 'X': ...")
                            // from a no-handler / expired / cancelled gate
                            // ("approval for tool 'X' unavailable: ...").
                            let ext_result = ExtFunctionResult::Error(MontyException::new(
                                ExcType::RuntimeError,
                                Some(outcome.script_message(&gate_action_name)),
                            ));
                            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                call.resume(ext_result, PrintWriter::CollectString(&mut stdout))
                            })) {
                                Ok(Ok(p)) => progress = p,
                                Ok(Err(e)) => {
                                    stdout.push_str(&format!("\nError: {e}"));
                                    return Ok(CodeExecutionResult {
                                        return_value: serde_json::Value::Null,
                                        stdout,
                                        action_results,
                                        events,
                                        need_approval: None,
                                        recursive_tokens,
                                        final_answer,
                                        failure: Some(CodeExecutionFailure::ToolError),
                                    });
                                }
                                Err(_) => {
                                    return Ok(CodeExecutionResult {
                                        return_value: serde_json::Value::Null,
                                        stdout: format!(
                                            "{stdout}\nVmPanic: Monty VM panicked during resume"
                                        ),
                                        action_results,
                                        events,
                                        need_approval: None,
                                        recursive_tokens,
                                        final_answer,
                                        failure: Some(CodeExecutionFailure::VmPanic),
                                    });
                                }
                            }
                            continue;
                        }

                        // Approved. Re-do preflight — the bridge installed
                        // any auto-approve preference before delivering the
                        // resolution, so policy now returns Allow.
                        let retry_preflight = preflight_action(
                            &gate_action_name,
                            &gate_parameters,
                            thread,
                            leases,
                            policy,
                            &execution_context,
                            capability_policies,
                            &gate_call_id,
                            &mut events,
                        )
                        .await;
                        match retry_preflight {
                            PreflightResult::Approved(lease) => {
                                let effects = effects.clone();
                                let name = gate_action_name.clone();
                                let params_clone = gate_parameters.clone();
                                let lease_clone = lease.clone();
                                let mut ctx = execution_context.clone();
                                ctx.current_call_id = Some(gate_call_id.clone());
                                // Carry the user's one-shot approval
                                // into the retry call so the host
                                // skips its per-call approval check.
                                ctx.call_approval_granted = true;
                                let ps =
                                    crate::types::event::summarize_params(&name, &gate_parameters);

                                let handle = tokio::spawn(async move {
                                    let execution_start = Instant::now();
                                    let result = effects
                                        .execute_action(&name, params_clone, &lease_clone, &ctx)
                                        .await;
                                    (result, execution_start.elapsed().as_millis() as u64)
                                });

                                pending_futures.insert(
                                    monty_call_id,
                                    PendingFuture::Tool {
                                        handle,
                                        action_name: gate_action_name,
                                        call_id: gate_call_id,
                                        lease_id: lease.id,
                                        params_summary: ps,
                                    },
                                );

                                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    call.resume_pending(PrintWriter::CollectString(&mut stdout))
                                })) {
                                    Ok(Ok(p)) => progress = p,
                                    Ok(Err(e)) => {
                                        stdout.push_str(&format!("\nError: {e}"));
                                        return Ok(CodeExecutionResult {
                                            return_value: serde_json::Value::Null,
                                            stdout,
                                            action_results,
                                            events,
                                            need_approval: None,
                                            recursive_tokens,
                                            final_answer,
                                            failure: Some(CodeExecutionFailure::ToolError),
                                        });
                                    }
                                    Err(_) => {
                                        return Ok(CodeExecutionResult {
                                            return_value: serde_json::Value::Null,
                                            stdout: format!(
                                                "{stdout}\nVmPanic: Monty VM panicked during resume_pending"
                                            ),
                                            action_results,
                                            events,
                                            need_approval: None,
                                            recursive_tokens,
                                            final_answer,
                                            failure: Some(CodeExecutionFailure::VmPanic),
                                        });
                                    }
                                }
                            }
                            PreflightResult::Denied(ext_result) => {
                                // Race: someone changed the lease/policy
                                // between approval and retry. Surface the
                                // error to Python so the script can handle
                                // it (or crash uncaught).
                                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    call.resume(ext_result, PrintWriter::CollectString(&mut stdout))
                                })) {
                                    Ok(Ok(p)) => progress = p,
                                    _ => {
                                        return Ok(CodeExecutionResult {
                                            return_value: serde_json::Value::Null,
                                            stdout,
                                            action_results,
                                            events,
                                            need_approval: None,
                                            recursive_tokens,
                                            final_answer,
                                            failure: Some(CodeExecutionFailure::ToolError),
                                        });
                                    }
                                }
                            }
                            PreflightResult::GatePaused(_) => {
                                // Policy still says approval needed even
                                // after user said yes. Treat as denial so
                                // we don't loop forever.
                                let ext_result = ExtFunctionResult::Error(MontyException::new(
                                    ExcType::RuntimeError,
                                    Some(format!(
                                        "tool '{gate_action_name}' still requires approval after resolution"
                                    )),
                                ));
                                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    call.resume(ext_result, PrintWriter::CollectString(&mut stdout))
                                })) {
                                    Ok(Ok(p)) => progress = p,
                                    _ => {
                                        return Ok(CodeExecutionResult {
                                            return_value: serde_json::Value::Null,
                                            stdout,
                                            action_results,
                                            events,
                                            need_approval: None,
                                            recursive_tokens,
                                            final_answer,
                                            failure: Some(CodeExecutionFailure::ToolError),
                                        });
                                    }
                                }
                            }
                        }
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
                                params_summary,
                            } => {
                                resolve_tool_future(
                                    handle,
                                    &action_name,
                                    &call_id,
                                    lease_id,
                                    params_summary,
                                    leases,
                                    effects,
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
                    resolve.resume(results, PrintWriter::CollectString(&mut stdout))
                })) {
                    Ok(Ok(p)) => progress = p,
                    Ok(Err(e)) => {
                        let pending_gate = take_pending_gate_stash();
                        stdout.push_str(&format!("\nError: {e}"));
                        return Ok(CodeExecutionResult {
                            return_value: serde_json::Value::Null,
                            stdout,
                            action_results,
                            events,
                            need_approval: pending_gate,
                            recursive_tokens,
                            final_answer,
                            failure: Some(classify_runtime_error(&e.to_string())),
                        });
                    }
                    Err(_) => {
                        return Ok(CodeExecutionResult {
                            return_value: serde_json::Value::Null,
                            stdout: format!(
                                "{stdout}\nVmPanic: Monty VM panicked during ResolveFutures resume"
                            ),
                            action_results,
                            events,
                            need_approval: None,
                            recursive_tokens,
                            final_answer,
                            failure: Some(CodeExecutionFailure::VmPanic),
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
                    lookup.resume(result, PrintWriter::CollectString(&mut stdout))
                })) {
                    Ok(Ok(p)) => progress = p,
                    Ok(Err(e)) => {
                        stdout.push_str(&format!("\nNameError: {e}"));
                        return Ok(CodeExecutionResult {
                            return_value: serde_json::Value::Null,
                            stdout,
                            action_results,
                            events,
                            need_approval: None,
                            recursive_tokens,
                            final_answer,
                            failure: Some(CodeExecutionFailure::NameLookup),
                        });
                    }
                    Err(_) => {
                        return Ok(CodeExecutionResult {
                            return_value: serde_json::Value::Null,
                            stdout: format!(
                                "{stdout}\nVmPanic: Monty VM panicked during name lookup"
                            ),
                            action_results,
                            events,
                            need_approval: None,
                            recursive_tokens,
                            final_answer,
                            failure: Some(CodeExecutionFailure::VmPanic),
                        });
                    }
                }
            }

            RunProgress::OsCall(os_call) => {
                // Clock reads (`datetime.now()`, `date.today()`) are not a
                // security concern — they don't touch the network, filesystem,
                // or environment. Monty surfaces them as dedicated OsFunction
                // variants rather than opaque syscalls, so we can answer them
                // directly instead of returning the blanket OSError. Anything
                // else still gets denied.
                let clock_reply: Option<ExtFunctionResult> = match os_call.function {
                    OsFunction::DateTimeNow => {
                        Some(ExtFunctionResult::Return(build_datetime_now(&os_call.args)))
                    }
                    OsFunction::DateToday => Some(ExtFunctionResult::Return(build_date_today())),
                    _ => None,
                };
                let reply = clock_reply.unwrap_or_else(|| {
                    debug!(function = ?os_call.function, "Monty: OS call denied");
                    ExtFunctionResult::Error(MontyException::new(
                        ExcType::OSError,
                        Some("OS operations are not permitted in CodeAct scripts".into()),
                    ))
                });
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    os_call.resume(reply, PrintWriter::CollectString(&mut stdout))
                })) {
                    Ok(Ok(p)) => progress = p,
                    Ok(Err(e)) => {
                        stdout.push_str(&format!("\nOSError: {e}"));
                        return Ok(CodeExecutionResult {
                            return_value: serde_json::Value::Null,
                            stdout,
                            action_results,
                            events,
                            need_approval: None,
                            recursive_tokens,
                            final_answer,
                            failure: Some(CodeExecutionFailure::OsDenied),
                        });
                    }
                    Err(_) => {
                        return Ok(CodeExecutionResult {
                            return_value: serde_json::Value::Null,
                            stdout: format!("{stdout}\nVmPanic: Monty VM panicked during OS call"),
                            action_results,
                            events,
                            need_approval: None,
                            recursive_tokens,
                            final_answer,
                            failure: Some(CodeExecutionFailure::VmPanic),
                        });
                    }
                }
            }
        }
    }
}

// ── Error classification ────────────────────────────────────

/// Classify a runtime error message into a failure category.
///
/// Parses the error text from Monty to distinguish between LLM logic bugs
/// (NameError, TypeError, etc.), resource limit hits, and Monty VM issues.
fn classify_runtime_error(error_msg: &str) -> CodeExecutionFailure {
    let lower = error_msg.to_ascii_lowercase();

    // Most specific checks first to avoid substring false positives.
    if lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("memory limit")
        || lower.contains("allocation limit")
        || lower.contains("out of fuel")
        || lower.contains("fuel exhausted")
        || lower.contains("resource limit")
    {
        CodeExecutionFailure::ResourceLimit
    } else if lower.contains("os operations are not permitted") || lower.contains("oserror") {
        CodeExecutionFailure::OsDenied
    } else if lower.contains("syntaxerror") {
        CodeExecutionFailure::SyntaxError
    } else {
        // NameError, TypeError, ValueError, AttributeError, IndexError,
        // KeyError, ModuleNotFoundError, NotImplementedError, etc.
        CodeExecutionFailure::RuntimeError
    }
}

/// Compute a short hash of Python code for dedup/correlation in events.
///
/// Uses FNV-1a (64-bit) which is stable across Rust versions, unlike
/// `DefaultHasher`. Not cryptographic — collision probability is ~2^-32
/// at typical usage levels, sufficient for dedup but not for security.
pub fn code_hash(code: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x00000100000001B3;
    let mut hash = FNV_OFFSET;
    for byte in code.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

// ── Pending future tracking ─────────────────────────────────

/// A deferred computation spawned as a tokio task, pending resolution
/// via `ResolveFutures`. Can be a tool execution or an LLM call.
enum PendingFuture {
    /// Tool action execution.
    ///
    /// We deliberately don't carry the call's `parameters` here: when
    /// the tool returns `EngineError::GatePaused`, the gate's own
    /// parameter snapshot (potentially safety-transformed) is the
    /// source of truth for the user-facing prompt and the inline
    /// retry. Caching the original would make it a misleading second
    /// source.
    Tool {
        handle: tokio::task::JoinHandle<(Result<ActionResult, EngineError>, u64)>,
        action_name: String,
        call_id: String,
        lease_id: crate::types::capability::LeaseId,
        params_summary: Option<String>,
    },
    /// LLM call (llm_query / llm_query_batched / rlm_query).
    Llm {
        handle: tokio::task::JoinHandle<(ExtFunctionResult, TokenUsage)>,
    },
}

impl PendingFuture {
    /// Pending future that resolves immediately to `None` with no token
    /// usage. Used for `FINAL` / `FINAL_VAR` so they can be `await`ed
    /// without raising "NoneType can't be awaited".
    fn ready_none() -> Self {
        let handle = tokio::spawn(async {
            (
                ExtFunctionResult::Return(MontyObject::None),
                TokenUsage::default(),
            )
        });
        PendingFuture::Llm { handle }
    }
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
    leases: &LeaseManager,
    policy: &PolicyEngine,
    context: &ThreadExecutionContext,
    capability_policies: &[crate::types::capability::PolicyRule],
    call_id: &str,
    events: &mut Vec<EventKind>,
) -> PreflightResult {
    let action_def = context
        .available_actions_snapshot
        .as_ref()
        .and_then(|actions| {
            actions
                .iter()
                .find(|action| action.matches_name(action_name))
        });
    if context.available_actions_snapshot.is_some() && action_def.is_none() {
        let error = format!("action '{action_name}' is not callable in this execution context");
        events.push(EventKind::ActionFailed {
            step_id: context.step_id,
            action_name: action_name.into(),
            call_id: call_id.into(),
            error: error.clone(),
            duration_ms: 0,
            params_summary: crate::types::event::summarize_params(action_name, params),
        });
        return PreflightResult::Denied(ExtFunctionResult::Error(MontyException::new(
            ExcType::RuntimeError,
            Some(error),
        )));
    }

    let lease = match leases.find_lease_for_action(thread.id, action_name).await {
        Some(l) => l,
        None => {
            events.push(EventKind::ActionFailed {
                step_id: context.step_id,
                action_name: action_name.into(),
                call_id: call_id.into(),
                error: format!("no lease for action '{action_name}'"),
                duration_ms: 0,
                params_summary: crate::types::event::summarize_params(action_name, params),
            });
            return PreflightResult::Denied(ExtFunctionResult::Error(MontyException::new(
                ExcType::RuntimeError,
                Some(format!("no lease for action '{action_name}'")),
            )));
        }
    };

    let canonical_action_name = action_def
        .as_ref()
        .map(|action| action.name.as_str())
        .unwrap_or(action_name);

    if let Some(action_def) = action_def {
        match policy.evaluate(action_def, &lease, capability_policies) {
            PolicyDecision::Deny { reason } => {
                events.push(EventKind::ActionFailed {
                    step_id: context.step_id,
                    action_name: action_name.into(),
                    call_id: call_id.into(),
                    error: reason.clone(),
                    duration_ms: 0,
                    params_summary: crate::types::event::summarize_params(action_name, params),
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
                        action_name: canonical_action_name.into(),
                        call_id: call_id.into(),
                        parameters: params.clone(),
                        resume_kind: crate::gate::ResumeKind::Approval { allow_always: true },
                        resume_output: None,
                        paused_lease: None,
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
    // `model` must be parsed explicitly — `extract_string_arg` coerces via
    // `monty_to_string`, which turns `MontyObject::None` into the literal
    // string "None" and stringifies non-string values, both of which would
    // silently route the call to an invalid model ID. Accept only str or None.
    let model_arg = match extract_optional_string_kwarg(args, kwargs, "model", 2) {
        Ok(v) => v,
        Err(err) => return err,
    };

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
        model: model_arg,
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

    // Positional/keyword layout (matches the documented signature
    // `llm_query_batched(prompts, context=None, model=None, models=None)`):
    //   arg 0 = prompts   (already extracted above)
    //   arg 1 = context
    //   arg 2 = model
    //   arg 3 = models
    // All three of context/model/models can also be passed by keyword.
    let context_arg = match extract_optional_string_kwarg(args, kwargs, "context", 1) {
        Ok(v) => v,
        Err(err) => return err,
    };

    // Optional model overrides:
    //   - `model="..."` applies the same model to every prompt
    //   - `models=[...]` is a parallel array (must match prompts length); use
    //     this to broadcast the same prompt across a council of models by
    //     passing `prompts=[same]*N, models=[m1, m2, ...]`. Within `models`,
    //     a `None` slot means "no override for this prompt" (the caller
    //     opted out of routing for that slot); the singular `model=` kwarg
    //     does NOT fill those slots, since mixing the two would be surprising.
    // See note in handle_llm_query: `model` must be parsed explicitly so that
    // `model=None` doesn't become the literal string "None".
    let single_model = match extract_optional_string_kwarg(args, kwargs, "model", 2) {
        Ok(v) => v,
        Err(err) => return err,
    };
    let models_kwarg = kwargs
        .iter()
        .find_map(|(k, v)| match k {
            MontyObject::String(key) if key == "models" => Some(v),
            _ => None,
        })
        .or_else(|| args.get(3));

    let models_list: Option<Vec<Option<String>>> = match models_kwarg {
        None | Some(MontyObject::None) => None,
        Some(MontyObject::List(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    MontyObject::String(s) => out.push(Some(s.clone())),
                    MontyObject::None => out.push(None),
                    other => {
                        return ExtFunctionResult::Error(MontyException::new(
                            ExcType::TypeError,
                            Some(format!(
                                "llm_query_batched(): models list entries must be str or None, got {other:?}"
                            )),
                        ));
                    }
                }
            }
            Some(out)
        }
        Some(other) => {
            return ExtFunctionResult::Error(MontyException::new(
                ExcType::TypeError,
                Some(format!(
                    "llm_query_batched(): `models` must be a list of str or None, got {other:?}"
                )),
            ));
        }
    };

    if let Some(ref ms) = models_list
        && ms.len() != prompts.len()
    {
        return ExtFunctionResult::Error(MontyException::new(
            ExcType::ValueError,
            Some(format!(
                "llm_query_batched(): models list length ({}) must match prompts length ({})",
                ms.len(),
                prompts.len()
            )),
        ));
    }

    let mut handles = Vec::with_capacity(prompts.len());
    for (i, prompt) in prompts.iter().enumerate() {
        let llm = Arc::clone(llm);
        let ctx = context_arg.clone();
        let prompt = prompt.clone();
        // If `models=` was provided, each slot is authoritative — `None` means
        // "no override for this prompt" and is NOT backfilled from `model=`.
        // Otherwise, fall back to the singular `model=` kwarg (or None).
        let model_override = match models_list.as_ref() {
            Some(ms) => ms[i].clone(),
            None => single_model.clone(),
        };
        let config = LlmCallConfig {
            force_text: true,
            model: model_override,
            ..LlmCallConfig::default()
        };
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
    gate_controller: &Arc<dyn crate::gate::GateController>,
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
        gate_controller.clone(),
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
                crate::runtime::messaging::ThreadOutcome::Failed { error, .. } => {
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

/// Maximum number of inline gate-await iterations for a single tool
/// call. The first attempt comes from the caller; this cap covers
/// retries triggered by post-approval policy still demanding approval
/// (e.g. a second gate kicks in after auto-approve was installed).
/// Three is enough for any plausible chain — a tool that gates more
/// than that is misbehaving and we'd rather surface a clean error
/// than spin forever.
///
/// Shared with Tier 0 (`structured::execute_with_inline_gate_retry`)
/// so both executors enforce the same upper bound.
pub(crate) const MAX_INLINE_GATE_RETRIES: usize = 3;

/// Inputs needed to drive one inline gate await.
struct InlineGate {
    gate_name: String,
    action_name: String,
    call_id: String,
    parameters: serde_json::Value,
    resume_kind: crate::gate::ResumeKind,
}

/// Drive an `Approval` gate to terminal resolution, retrying the
/// action up to [`MAX_INLINE_GATE_RETRIES`] times if the post-approval
/// retry itself returns `GatePaused`.
///
/// Centralizes Tier 1's gate handling so the async output path and the
/// sync preflight path emit consistent events / error messages — and
/// so a misbehaving tool (gates repeatedly after approval) produces a
/// bounded `RuntimeError` instead of leaking the legacy
/// "execution paused by gate" message.
#[allow(clippy::too_many_arguments)]
async fn drive_inline_gate(
    mut gate: InlineGate,
    leases: &LeaseManager,
    effects: &Arc<dyn EffectExecutor>,
    context: &ThreadExecutionContext,
    action_results: &mut Vec<ActionResult>,
    events: &mut Vec<EventKind>,
    params_summary: Option<String>,
) -> ExtFunctionResult {
    for _ in 0..MAX_INLINE_GATE_RETRIES {
        let resolution = context
            .gate_controller
            .pause(crate::gate::GatePauseRequest {
                thread_id: context.thread_id,
                user_id: context.user_id.clone(),
                gate_name: gate.gate_name.clone(),
                action_name: gate.action_name.clone(),
                call_id: gate.call_id.clone(),
                parameters: gate.parameters.clone(),
                resume_kind: gate.resume_kind.clone(),
                conversation_id: context.conversation_id,
            })
            .await;

        if let Some(outcome) = denial_outcome_for_resolution(&resolution) {
            // Cancelled+Authentication → unwind via the legacy
            // `RuntimeError("execution paused by gate ...")` so the
            // outer orchestrator can produce `ThreadOutcome::GatePaused`
            // and missions can transition to Paused. Cancelled here
            // means the controller can't resolve the auth inline (no
            // OAuth wiring) — the legacy unwind path is the right
            // fallback. Denied / explicit user-cancel remain failures.
            if matches!(resolution, crate::gate::GateResolution::Cancelled)
                && matches!(
                    gate.resume_kind,
                    crate::gate::ResumeKind::Authentication { .. }
                )
            {
                // Stash the original gate so `execute_code`'s exit can
                // surface it as `need_approval`. Without this, Tier 1
                // mission child threads silently swallow the gate and
                // the cron keeps re-firing the mission (#3133 ghost
                // fire). Best-effort — if the task-local isn't in
                // scope (theoretically impossible since we always run
                // inside `execute_code_with_skills`'s scope, but
                // defensive), `try_with` no-ops.
                let _ = PENDING_GATE_STASH.try_with(|cell| {
                    *cell.borrow_mut() =
                        Some(crate::runtime::messaging::ThreadOutcome::GatePaused {
                            gate_name: gate.gate_name.clone(),
                            action_name: gate.action_name.clone(),
                            call_id: gate.call_id.clone(),
                            parameters: gate.parameters.clone(),
                            resume_kind: gate.resume_kind.clone(),
                            resume_output: None,
                            paused_lease: None,
                        });
                });
                return ExtFunctionResult::Error(MontyException::new(
                    ExcType::RuntimeError,
                    Some(format!("execution paused by gate '{}'", gate.gate_name)),
                ));
            }
            events.push(EventKind::ActionFailed {
                step_id: context.step_id,
                action_name: gate.action_name.clone(),
                call_id: gate.call_id.clone(),
                error: outcome.event_error(),
                duration_ms: 0,
                params_summary,
            });
            return ExtFunctionResult::Error(MontyException::new(
                ExcType::RuntimeError,
                Some(outcome.script_message(&gate.action_name)),
            ));
        }

        // Approved. Re-acquire a lease use and retry the action. The
        // bridge installed any auto-approve preference before
        // delivering the resolution, so policy now returns Allow.
        //
        // Note: `find_and_consume` may select a different lease than
        // the originally-refunded one if multiple grants cover this
        // action. That's fine for the use-counter contract; if leases
        // ever carry per-grant identity (credential bindings) this
        // assumption needs to be re-evaluated.
        let lease = match leases
            .find_and_consume(context.thread_id, &gate.action_name)
            .await
        {
            Ok(l) => l,
            Err(e) => {
                return ExtFunctionResult::Error(MontyException::new(
                    ExcType::RuntimeError,
                    Some(format!("lease unavailable after approval: {e}")),
                ));
            }
        };
        // Carry the user's one-shot approval into the retry call so
        // the host's `EffectExecutor` skips the
        // `ApprovalRequirement::Always` / AskEachTime gate that would
        // otherwise fire again. Mirrors the legacy
        // `execute_resolved_pending_action(approval_already_granted=true)`
        // path. Scoped to this single call by `current_call_id`.
        let mut retry_ctx = context.clone();
        retry_ctx.current_call_id = Some(gate.call_id.clone());
        retry_ctx.call_approval_granted = true;
        let retry_start = Instant::now();
        let retry_result = effects
            .execute_action(
                &gate.action_name,
                gate.parameters.clone(),
                &lease,
                &retry_ctx,
            )
            .await;
        let retry_duration_ms = retry_start.elapsed().as_millis() as u64;

        match retry_result {
            Ok(result) => {
                if result.is_error {
                    let error_msg = result
                        .output
                        .get("error")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                        .unwrap_or_else(|| result.output.to_string());
                    let duration_ms = result.duration.as_millis() as u64;
                    events.push(EventKind::ActionFailed {
                        step_id: context.step_id,
                        action_name: gate.action_name.clone(),
                        call_id: gate.call_id.clone(),
                        error: error_msg,
                        duration_ms: if duration_ms > 0 {
                            duration_ms
                        } else {
                            retry_duration_ms
                        },
                        params_summary,
                    });
                } else {
                    events.push(EventKind::ActionExecuted {
                        step_id: context.step_id,
                        action_name: gate.action_name.clone(),
                        call_id: gate.call_id.clone(),
                        duration_ms: result.duration.as_millis() as u64,
                        params_summary,
                    });
                }
                let monty_val = json_to_monty(&result.output);
                action_results.push(result);
                return ExtFunctionResult::Return(monty_val);
            }
            Err(EngineError::GatePaused {
                gate_name,
                action_name,
                call_id,
                parameters,
                resume_kind,
                ..
            }) if matches!(
                *resume_kind,
                crate::gate::ResumeKind::Approval { .. }
                    | crate::gate::ResumeKind::Authentication { .. }
            ) =>
            {
                // Refund the use we just consumed — the next loop
                // iteration will pause and re-consume on resolution.
                let _ = leases.refund_use(lease.id).await;
                events.push(EventKind::ApprovalRequested {
                    action_name: action_name.clone(),
                    call_id: call_id.clone(),
                    parameters: Some((*parameters).clone()),
                    description: None,
                    allow_always: match *resume_kind {
                        crate::gate::ResumeKind::Approval { allow_always } => Some(allow_always),
                        _ => None,
                    },
                    gate_name: Some(gate_name.clone()),
                    params_summary: params_summary.clone(),
                });
                gate = InlineGate {
                    gate_name,
                    action_name,
                    call_id,
                    parameters: *parameters,
                    resume_kind: *resume_kind,
                };
                continue;
            }
            Err(e) => {
                events.push(EventKind::ActionFailed {
                    step_id: context.step_id,
                    action_name: gate.action_name.clone(),
                    call_id: gate.call_id.clone(),
                    error: e.to_string(),
                    duration_ms: retry_duration_ms,
                    params_summary,
                });
                return ExtFunctionResult::Error(MontyException::new(
                    ExcType::RuntimeError,
                    Some(e.to_string()),
                ));
            }
        }
    }

    // Retry budget exhausted — the tool kept gating after every
    // approval. Surface as a typed error so the script can react;
    // the user already approved this many times in a row, no point
    // asking again.
    events.push(EventKind::ActionFailed {
        step_id: context.step_id,
        action_name: gate.action_name.clone(),
        call_id: gate.call_id.clone(),
        error: format!("tool kept gating after {MAX_INLINE_GATE_RETRIES} approvals"),
        duration_ms: 0,
        params_summary,
    });
    ExtFunctionResult::Error(MontyException::new(
        ExcType::RuntimeError,
        Some(format!(
            "tool '{}' still requires approval after {MAX_INLINE_GATE_RETRIES} retries",
            gate.action_name
        )),
    ))
}

/// Resolve a pending tool execution future.
///
/// Deliberately does NOT take the original `parameters`: when a tool
/// returns `EngineError::GatePaused`, the gate carries its own
/// parameter snapshot (possibly transformed by the safety layer) and
/// that's what we surface to the user. Threading the original
/// parameters through here would make them a misleading second
/// source of truth.
#[allow(clippy::too_many_arguments)]
async fn resolve_tool_future(
    handle: tokio::task::JoinHandle<(Result<ActionResult, EngineError>, u64)>,
    action_name: &str,
    call_id: &str,
    lease_id: crate::types::capability::LeaseId,
    params_summary: Option<String>,
    leases: &LeaseManager,
    effects: &Arc<dyn EffectExecutor>,
    context: &ThreadExecutionContext,
    action_results: &mut Vec<ActionResult>,
    events: &mut Vec<EventKind>,
) -> ExtFunctionResult {
    match handle.await {
        Ok((Ok(result), execution_duration_ms)) => {
            // If the effect adapter wrapped a tool error as an Ok(ActionResult)
            // with is_error=true (current convention in
            // `EffectBridgeAdapter::execute_action_internal`), surface it as
            // ActionFailed so traces, observers, and approval flows see the
            // failure correctly. Without this, every wrapped error looked like
            // a successful tool call to downstream consumers.
            if result.is_error {
                let error_msg = result
                    .output
                    .get("error")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| result.output.to_string());
                let duration_ms = result.duration.as_millis() as u64;
                events.push(EventKind::ActionFailed {
                    step_id: context.step_id,
                    action_name: action_name.into(),
                    call_id: call_id.into(),
                    error: error_msg,
                    duration_ms: if duration_ms > 0 {
                        duration_ms
                    } else {
                        execution_duration_ms
                    },
                    params_summary,
                });
            } else {
                events.push(EventKind::ActionExecuted {
                    step_id: context.step_id,
                    action_name: action_name.into(),
                    call_id: call_id.into(),
                    duration_ms: result.duration.as_millis() as u64,
                    params_summary,
                });
            }
            let monty_val = json_to_monty(&result.output);
            action_results.push(result);
            ExtFunctionResult::Return(monty_val)
        }
        Ok((
            Err(EngineError::GatePaused {
                gate_name,
                action_name: gate_action_name,
                call_id: gate_call_id,
                parameters: gate_parameters,
                resume_kind,
                ..
            }),
            _,
        )) => {
            let _ = leases.refund_use(lease_id).await;
            events.push(EventKind::ApprovalRequested {
                action_name: gate_action_name.clone(),
                call_id: gate_call_id.clone(),
                parameters: Some((*gate_parameters).clone()),
                description: None,
                allow_always: match *resume_kind {
                    crate::gate::ResumeKind::Approval { allow_always } => Some(allow_always),
                    _ => None,
                },
                gate_name: Some(gate_name.clone()),
                params_summary: params_summary.clone(),
            });

            // External resume kinds keep the legacy re-entry path —
            // their resolution installs callback-payload state that
            // can't be handed back to a suspended call. Approval and
            // Authentication both go through `drive_inline_gate`:
            // Approval resolves on user click, Authentication resolves
            // when `bridge::resolve_inline_gates_for_credential` (the
            // OAuth-callback hook from #3133 half-2) delivers
            // `GateResolution::Approved` to the parked controller.
            // (Mission-scoped resumes go through
            // `bridge::resume_paused_missions_for_credential` — that's
            // a separate path for background missions whose child
            // threads were paused on the same gate.) In both
            // inline-await cases the action retries inline and the
            // script continues without unwinding.
            if !matches!(
                *resume_kind,
                crate::gate::ResumeKind::Approval { .. }
                    | crate::gate::ResumeKind::Authentication { .. }
            ) {
                return ExtFunctionResult::Error(MontyException::new(
                    ExcType::RuntimeError,
                    Some(format!("execution paused by gate '{gate_name}'")),
                ));
            }

            drive_inline_gate(
                InlineGate {
                    gate_name,
                    action_name: gate_action_name,
                    call_id: gate_call_id,
                    parameters: *gate_parameters,
                    resume_kind: *resume_kind,
                },
                leases,
                effects,
                context,
                action_results,
                events,
                params_summary,
            )
            .await
        }
        Ok((Err(e), execution_duration_ms)) => {
            events.push(EventKind::ActionFailed {
                step_id: context.step_id,
                action_name: action_name.into(),
                call_id: call_id.into(),
                error: e.to_string(),
                duration_ms: execution_duration_ms,
                params_summary,
            });
            action_results.push(ActionResult {
                call_id: call_id.into(),
                action_name: action_name.into(),
                output: serde_json::json!({"error": e.to_string()}),
                is_error: true,
                duration: Duration::from_millis(execution_duration_ms),
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

/// Strict optional-string extractor for arguments where silent coercion is
/// dangerous (e.g. `model=` — passing the wrong type should NOT become an
/// unintended model ID). Returns:
///   - `Ok(None)` when the argument is missing or explicitly `None`
///   - `Ok(Some(s))` when the argument is a string
///   - `Err(TypeError)` for any other type
fn extract_optional_string_kwarg(
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
    name: &str,
    position: usize,
) -> Result<Option<String>, ExtFunctionResult> {
    let raw = kwargs
        .iter()
        .find_map(|(k, v)| match k {
            MontyObject::String(key) if key == name => Some(v),
            _ => None,
        })
        .or_else(|| args.get(position));

    match raw {
        None | Some(MontyObject::None) => Ok(None),
        Some(MontyObject::String(s)) => Ok(Some(s.clone())),
        Some(other) => Err(ExtFunctionResult::Error(MontyException::new(
            ExcType::TypeError,
            Some(format!("`{name}` must be a string or None, got {other:?}")),
        ))),
    }
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
    use crate::types::capability::{
        ActionDef, CapabilityLease, EffectType, GrantedActions, ModelToolSurface,
    };
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
            _context: &ThreadExecutionContext,
        ) -> Result<Vec<ActionDef>, EngineError> {
            Ok(self.actions.clone())
        }

        async fn available_capabilities(
            &self,
            _: &[CapabilityLease],
            _: &ThreadExecutionContext,
        ) -> Result<Vec<crate::types::capability::CapabilitySummary>, EngineError> {
            Ok(vec![])
        }
    }

    fn test_action(name: &str) -> ActionDef {
        ActionDef {
            name: name.into(),
            description: "Test tool".into(),
            parameters_schema: serde_json::json!({"type": "object"}),
            effects: vec![EffectType::ReadLocal],
            requires_approval: false,
            model_tool_surface: ModelToolSurface::FullSchema,
            discovery: None,
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
            thread_goal: Some(thread.goal.clone()),
            available_actions_snapshot: None,
            available_action_inventory_snapshot: None,
            gate_controller: crate::gate::CancellingGateController::arc(),
            call_approval_granted: false,
            conversation_id: None,
        }
    }

    #[tokio::test]
    async fn preflight_rejects_actions_outside_callable_snapshot() {
        let thread = make_test_thread();
        let leases = LeaseManager::new();
        let policy = PolicyEngine::new();
        let mut ctx = make_exec_context(&thread);
        ctx.available_actions_snapshot = Some(Arc::from(vec![test_action("tool_info")]));
        leases
            .grant(thread.id, "tools", GrantedActions::All, None, Some(1))
            .await
            .expect("grant wildcard lease");

        let mut events = Vec::new();
        let result = preflight_action(
            "gmail_send",
            &serde_json::json!({"to": "user@example.com"}),
            &thread,
            &leases,
            &policy,
            &ctx,
            &[],
            "call-1",
            &mut events,
        )
        .await;

        assert!(matches!(result, PreflightResult::Denied(_)));
        assert!(matches!(
            events.as_slice(),
            [EventKind::ActionFailed { action_name, error, .. }]
                if action_name == "gmail_send"
                    && error.contains("not callable in this execution context")
        ));
        assert!(
            leases
                .find_lease_for_action(thread.id, "gmail_send")
                .await
                .is_some(),
            "denied snapshot misses must not consume the lease"
        );
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

    struct SnapshotAwareToolInfoEffects;

    #[async_trait::async_trait]
    impl EffectExecutor for SnapshotAwareToolInfoEffects {
        async fn execute_action(
            &self,
            action_name: &str,
            parameters: serde_json::Value,
            _lease: &CapabilityLease,
            ctx: &ThreadExecutionContext,
        ) -> Result<ActionResult, EngineError> {
            let output = if action_name == "tool_info" {
                let requested = parameters
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                match ctx
                    .available_action_inventory_snapshot
                    .as_ref()
                    .and_then(|inventory| {
                        inventory
                            .inline
                            .iter()
                            .find(|action| action.matches_name(requested))
                    }) {
                    Some(action) => {
                        if parameters.get("detail").and_then(|value| value.as_str())
                            == Some("schema")
                        {
                            serde_json::json!({
                                "name": action.name.clone(),
                                "schema": action.parameters_schema.clone()
                            })
                        } else {
                            serde_json::json!({
                                "name": action.name.clone(),
                                "summary": {
                                    "always_required": ["name", "goal", "cadence"]
                                }
                            })
                        }
                    }
                    None => serde_json::json!({"error": "missing inventory snapshot"}),
                }
            } else {
                serde_json::json!({"error": format!("unexpected action '{action_name}'")})
            };

            Ok(ActionResult {
                call_id: String::new(),
                action_name: action_name.to_string(),
                is_error: output.get("error").is_some(),
                output,
                duration: Duration::from_millis(1),
            })
        }

        async fn available_actions(
            &self,
            _leases: &[CapabilityLease],
            _context: &ThreadExecutionContext,
        ) -> Result<Vec<ActionDef>, EngineError> {
            Ok(self
                .available_action_inventory(_leases, _context)
                .await?
                .inline)
        }

        async fn available_action_inventory(
            &self,
            _leases: &[CapabilityLease],
            _context: &ThreadExecutionContext,
        ) -> Result<crate::types::capability::ActionInventory, EngineError> {
            Ok(crate::types::capability::ActionInventory {
                inline: vec![
                    test_action("tool_info"),
                    ActionDef {
                        name: "mission_create".into(),
                        description: "Create a mission".into(),
                        parameters_schema: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "name": {"type": "string"},
                                "goal": {"type": "string"},
                                "cadence": {"type": "string"}
                            },
                            "required": ["name", "goal", "cadence"]
                        }),
                        effects: vec![EffectType::WriteLocal],
                        requires_approval: false,
                        model_tool_surface: ModelToolSurface::CompactToolInfo,
                        discovery: None,
                    },
                ],
                discoverable: Vec::new(),
            })
        }

        async fn available_capabilities(
            &self,
            _: &[CapabilityLease],
            _: &ThreadExecutionContext,
        ) -> Result<Vec<crate::types::capability::CapabilitySummary>, EngineError> {
            Ok(vec![])
        }
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
            result.failure.is_none(),
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
        assert!(result.failure.is_none());
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
        assert!(result.failure.is_none(), "stdout: {}", result.stdout);
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
        assert!(result.failure.is_none(), "stdout: {}", result.stdout);
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
            result.failure.is_some(),
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
        assert!(result.failure.is_none());
    }

    // ── `await FINAL(...)` is tolerated ────────────────────
    //
    // LLMs frequently emit `await FINAL(answer)` by analogy with
    // async tool calls. Prior to the `ready_none` pending future,
    // this raised "TypeError: 'NoneType' object can't be awaited"
    // and the answer was lost. Both forms must now succeed.

    #[tokio::test]
    async fn final_supports_await() {
        let thread = make_test_thread();
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(vec![], vec![]));

        let code = r#"
await FINAL("hello from await")
"#;

        let result = run_code(code, effects, &thread).await.unwrap();
        assert_eq!(
            result.final_answer.as_deref(),
            Some("hello from await"),
            "stdout: {}",
            result.stdout
        );
        assert!(result.failure.is_none(), "stdout: {}", result.stdout);
    }

    #[tokio::test]
    async fn final_var_supports_await() {
        let thread = make_test_thread();
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(vec![], vec![]));

        let code = r#"
summary = "computed answer"
await FINAL_VAR("summary")
"#;

        let result = run_code(code, effects, &thread).await.unwrap();
        assert_eq!(
            result.final_answer.as_deref(),
            Some("[FINAL_VAR: summary]"),
            "stdout: {}",
            result.stdout
        );
        assert!(result.failure.is_none(), "stdout: {}", result.stdout);
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
        assert!(result.failure.is_none(), "stdout: {}", result.stdout);
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
        assert!(result.failure.is_none(), "stdout: {}", result.stdout);
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
        assert!(result.failure.is_none(), "stdout: {}", result.stdout);
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
                r.failure.is_some() || r.stdout.contains("Error") || r.stdout.contains("limit"),
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
                r.failure.is_some() || r.stdout.contains("Error") || r.stdout.contains("limit"),
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
        assert!(result.failure.is_some(), "syntax error should set failure");
        assert!(
            result.stdout.contains("SyntaxError") || result.stdout.contains("Error"),
            "should contain SyntaxError, got: {}",
            result.stdout,
        );
    }

    // ── Additional sandbox security negative tests ─────────────

    /// rlm_query() at max depth must be refused with a clear error.
    #[tokio::test]
    async fn sandbox_enforces_rlm_query_depth_limit() {
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(vec![], vec![]));
        let mut thread = make_test_thread();
        // Set depth at max — rlm_query should refuse to recurse further.
        thread.config.depth = 2;
        thread.config.max_depth = 2;

        let code = r#"
try:
    result = await rlm_query(prompt="nested call")
    FINAL("ESCAPED: " + str(result))
except Exception as e:
    FINAL("blocked: " + str(e))
"#;
        let leases = LeaseManager::new();
        let policy = PolicyEngine::new();
        let ctx = make_exec_context(&thread);
        leases
            .grant(thread.id, "tools", GrantedActions::All, None, None)
            .await
            .unwrap();

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
            "rlm_query should be blocked at max depth, got: {answer}",
        );
        assert!(
            answer.contains("depth limit"),
            "error should mention depth limit, got: {answer}",
        );
    }

    /// FINAL() payloads must be captured literally, not interpreted.
    #[tokio::test]
    async fn sandbox_rejects_final_injection() {
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(vec![], vec![]));
        let thread = make_test_thread();

        // Attempt to embed code-like content in FINAL payload.
        let code = r#"
FINAL("'); import os; os.system('echo pwned'); FINAL('clean")
"#;
        let result = run_code(code, effects.clone(), &thread).await.unwrap();
        let answer = result.final_answer.as_deref().unwrap_or("");
        // The payload should be captured as a literal string, not executed.
        assert!(
            answer.contains("import os"),
            "FINAL should capture the payload literally, got: {answer}",
        );
        assert!(
            !result.stdout.contains("pwned"),
            "injected os.system should not execute, stdout: {}",
            result.stdout,
        );

        // Multiple FINAL calls — last one wins (reassignment semantics).
        // This is safe because FINAL() just sets a variable, not a control flow exit.
        let code2 = r#"
FINAL("first")
FINAL("second")
"#;
        let result2 = run_code(code2, effects, &thread).await.unwrap();
        assert!(
            result2.final_answer.is_some(),
            "FINAL should capture an answer even with multiple calls",
        );
    }

    /// Special characters in tool names must not bypass lease enforcement.
    #[tokio::test]
    async fn sandbox_rejects_tool_name_injection() {
        let effects: Arc<dyn EffectExecutor> =
            Arc::new(MockEffects::new(vec![test_action("safe_tool")], vec![]));
        let thread = make_test_thread();
        let leases = LeaseManager::new();
        let policy = PolicyEngine::new();
        let ctx = make_exec_context(&thread);

        // Only grant lease for "safe_tool".
        leases
            .grant(
                thread.id,
                "tools",
                GrantedActions::Specific(vec!["safe_tool".into()]),
                None,
                None,
            )
            .await
            .unwrap();

        // Attempt dynamic name construction to bypass lease check.
        let code = r#"
try:
    name = "safe" + "_" + "tool; shell"
    fn = globals().get(name)
    if fn:
        result = await fn()
        FINAL("ESCAPED: " + str(result))
    else:
        FINAL("not_found")
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
            "dynamic name construction should not bypass leases, got: {answer}",
        );
        // Verify the expected outcome: name lookup found nothing callable.
        assert!(
            answer == "not_found" || answer.starts_with("blocked:"),
            "expected 'not_found' or 'blocked:*', got: {answer}",
        );
    }

    /// Mutations to the `context` Python variable must not affect Rust thread state.
    #[tokio::test]
    async fn sandbox_context_variable_is_not_mutable() {
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(vec![], vec![]));
        let mut thread = make_test_thread();
        thread.add_message(crate::types::message::ThreadMessage::user(
            "original message",
        ));
        let original_count = thread.messages.len();
        let original_goal = thread.goal.clone();

        let code = r#"
# Attempt to mutate the context variable
context.append({"role": "User", "content": "injected message"})
if len(context) > 0:
    context[0]["content"] = "TAMPERED"
# Also try to mutate goal
goal = "HIJACKED GOAL"
FINAL("mutated")
"#;
        let _result = run_code(code, effects, &thread).await.unwrap();
        // Monty operates on copies — mutations must not propagate back to Rust.
        assert_eq!(
            thread.messages.len(),
            original_count,
            "thread messages should not be mutated by Python code",
        );
        assert_eq!(
            thread.goal, original_goal,
            "thread goal should not be mutated by Python code",
        );
    }

    /// Deeply recursive Python functions must terminate, not crash the process.
    #[tokio::test]
    async fn sandbox_handles_deep_recursion() {
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects::new(vec![], vec![]));
        let thread = make_test_thread();

        let code = r#"
def recurse(n):
    return recurse(n + 1)
try:
    recurse(0)
    FINAL("ESCAPED: infinite recursion completed")
except Exception as e:
    FINAL("blocked: " + type(e).__name__)
"#;
        let result = run_code(code, effects, &thread).await;
        // Must not panic — either returns an error or the VM is killed by limits.
        match result {
            Ok(r) => {
                let answer = r.final_answer.as_deref().unwrap_or("");
                assert!(
                    !answer.starts_with("ESCAPED"),
                    "infinite recursion should be caught, got: {answer}",
                );
            }
            Err(_) => {
                // VM killed by resource limits — acceptable.
            }
        }
    }

    /// validate_python_syntax rejects broken code and accepts valid code.
    #[test]
    fn validate_syntax_rejects_broken_code() {
        assert!(validate_python_syntax("def f(\n").is_err());
        assert!(validate_python_syntax("x = 1\ny = 2\n").is_ok());
        // Empty input is valid Python (empty module).
        assert!(validate_python_syntax("").is_ok());
        // Unicode identifiers are valid Python 3.
        assert!(validate_python_syntax("café = 1\n").is_ok());
        // Oversized input is rejected before parsing.
        let oversized = "x = 1\n".repeat(50_000);
        let err = validate_python_syntax(&oversized).expect_err("oversized");
        assert!(
            err.contains("too large"),
            "expected size-cap error, got: {err}"
        );
        // Error messages contain "syntax error" prefix.
        let err = validate_python_syntax("def :\n").expect_err("syntax");
        assert!(
            err.starts_with("syntax error"),
            "expected 'syntax error' prefix, got: {err}"
        );
    }

    // ── llm_query model parameter plumbing ─────────────────────

    /// LLM backend that records every call's model + prompt for assertions.
    struct CapturingLlm {
        calls: tokio::sync::Mutex<Vec<(Option<String>, String)>>,
    }

    impl CapturingLlm {
        fn new() -> Self {
            Self {
                calls: tokio::sync::Mutex::new(Vec::new()),
            }
        }
    }
    #[async_trait::async_trait]
    impl crate::traits::llm::LlmBackend for CapturingLlm {
        fn model_name(&self) -> &str {
            "capturing"
        }

        async fn complete(
            &self,
            messages: &[crate::types::message::ThreadMessage],
            _actions: &[ActionDef],
            config: &crate::traits::llm::LlmCallConfig,
        ) -> Result<crate::traits::llm::LlmOutput, EngineError> {
            let user_prompt = messages
                .iter()
                .rev()
                .find(|m| matches!(m.role, crate::types::message::MessageRole::User))
                .map(|m| m.content.clone())
                .unwrap_or_default();
            self.calls
                .lock()
                .await
                .push((config.model.clone(), user_prompt.clone()));
            Ok(crate::traits::llm::LlmOutput {
                response: crate::types::step::LlmResponse::Text(format!(
                    "ack:{}:{user_prompt}",
                    config.model.as_deref().unwrap_or("default")
                )),
                usage: crate::types::step::TokenUsage::default(),
            })
        }
    }

    #[tokio::test]
    async fn llm_query_forwards_model_kwarg() {
        let llm = Arc::new(CapturingLlm::new());
        let mut tokens = crate::types::step::TokenUsage::default();
        let result = handle_llm_query(
            &[],
            &[
                (
                    MontyObject::String("prompt".into()),
                    MontyObject::String("what is 2+2?".into()),
                ),
                (
                    MontyObject::String("model".into()),
                    MontyObject::String("gpt-4o".into()),
                ),
            ],
            &(Arc::clone(&llm) as Arc<dyn crate::traits::llm::LlmBackend>),
            &mut tokens,
        )
        .await;

        match result {
            ExtFunctionResult::Return(MontyObject::String(s)) => {
                assert!(s.contains("gpt-4o"), "got: {s}");
            }
            other => panic!("expected string return, got {other:?}"),
        }

        let calls = llm.calls.lock().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0.as_deref(), Some("gpt-4o"));
        assert_eq!(calls[0].1, "what is 2+2?");
    }

    #[tokio::test]
    async fn llm_query_without_model_passes_none() {
        let llm = Arc::new(CapturingLlm::new());
        let mut tokens = crate::types::step::TokenUsage::default();
        let _ = handle_llm_query(
            &[MontyObject::String("hello".into())],
            &[],
            &(Arc::clone(&llm) as Arc<dyn crate::traits::llm::LlmBackend>),
            &mut tokens,
        )
        .await;

        let calls = llm.calls.lock().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, None);
    }

    #[tokio::test]
    async fn llm_query_batched_broadcasts_with_models_list() {
        let llm = Arc::new(CapturingLlm::new());
        let mut tokens = crate::types::step::TokenUsage::default();
        let prompts = MontyObject::List(vec![
            MontyObject::String("Q".into()),
            MontyObject::String("Q".into()),
            MontyObject::String("Q".into()),
        ]);
        let models = MontyObject::List(vec![
            MontyObject::String("gpt-4o".into()),
            MontyObject::String("claude-sonnet-4-20250514".into()),
            MontyObject::String("llama-3.1-70b-instruct".into()),
        ]);
        let result = handle_llm_query_batched(
            &[prompts],
            &[(MontyObject::String("models".into()), models)],
            &(Arc::clone(&llm) as Arc<dyn crate::traits::llm::LlmBackend>),
            &mut tokens,
        )
        .await;

        match result {
            ExtFunctionResult::Return(MontyObject::List(items)) => {
                assert_eq!(items.len(), 3);
            }
            other => panic!("expected list return, got {other:?}"),
        }

        let mut calls = llm.calls.lock().await;
        calls.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].0.as_deref(), Some("claude-sonnet-4-20250514"));
        assert_eq!(calls[1].0.as_deref(), Some("gpt-4o"));
        assert_eq!(calls[2].0.as_deref(), Some("llama-3.1-70b-instruct"));
    }

    #[tokio::test]
    async fn llm_query_batched_single_model_applies_to_all() {
        let llm = Arc::new(CapturingLlm::new());
        let mut tokens = crate::types::step::TokenUsage::default();
        let prompts = MontyObject::List(vec![
            MontyObject::String("a".into()),
            MontyObject::String("b".into()),
        ]);
        let _ = handle_llm_query_batched(
            &[prompts],
            &[(
                MontyObject::String("model".into()),
                MontyObject::String("gpt-4o".into()),
            )],
            &(Arc::clone(&llm) as Arc<dyn crate::traits::llm::LlmBackend>),
            &mut tokens,
        )
        .await;

        let calls = llm.calls.lock().await;
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|(m, _)| m.as_deref() == Some("gpt-4o")));
    }

    #[tokio::test]
    async fn llm_query_model_none_kwarg_is_no_override_not_literal_none_string() {
        // Regression: `extract_string_arg` would have coerced
        // MontyObject::None to the literal string "None", silently routing
        // every model=None call to an invalid model ID. Must stay None.
        let llm = Arc::new(CapturingLlm::new());
        let mut tokens = crate::types::step::TokenUsage::default();
        let _ = handle_llm_query(
            &[],
            &[
                (
                    MontyObject::String("prompt".into()),
                    MontyObject::String("hi".into()),
                ),
                (MontyObject::String("model".into()), MontyObject::None),
            ],
            &(Arc::clone(&llm) as Arc<dyn crate::traits::llm::LlmBackend>),
            &mut tokens,
        )
        .await;

        let calls = llm.calls.lock().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, None);
    }

    #[tokio::test]
    async fn llm_query_rejects_non_string_model_kwarg() {
        let llm = Arc::new(CapturingLlm::new());
        let mut tokens = crate::types::step::TokenUsage::default();
        let result = handle_llm_query(
            &[],
            &[
                (
                    MontyObject::String("prompt".into()),
                    MontyObject::String("hi".into()),
                ),
                (MontyObject::String("model".into()), MontyObject::Int(42)),
            ],
            &(Arc::clone(&llm) as Arc<dyn crate::traits::llm::LlmBackend>),
            &mut tokens,
        )
        .await;

        assert!(matches!(result, ExtFunctionResult::Error(_)));
        assert!(llm.calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn llm_query_batched_single_model_none_kwarg_is_no_override() {
        let llm = Arc::new(CapturingLlm::new());
        let mut tokens = crate::types::step::TokenUsage::default();
        let prompts = MontyObject::List(vec![
            MontyObject::String("a".into()),
            MontyObject::String("b".into()),
        ]);
        let _ = handle_llm_query_batched(
            &[prompts],
            &[(MontyObject::String("model".into()), MontyObject::None)],
            &(Arc::clone(&llm) as Arc<dyn crate::traits::llm::LlmBackend>),
            &mut tokens,
        )
        .await;

        let calls = llm.calls.lock().await;
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|(m, _)| m.is_none()));
    }

    #[tokio::test]
    async fn llm_query_batched_honors_positional_context_and_model() {
        // Regression: `context`, `model`, and `models` used to be kwarg-only.
        // A positional call matching the documented signature
        // `llm_query_batched(prompts, context=None, model=None, models=None)`
        // silently dropped the model, violating the preamble.
        let llm = Arc::new(CapturingLlm::new());
        let mut tokens = crate::types::step::TokenUsage::default();
        let result = handle_llm_query_batched(
            &[
                MontyObject::List(vec![
                    MontyObject::String("a".into()),
                    MontyObject::String("b".into()),
                ]),
                MontyObject::String("shared context".into()), // position 1: context
                MontyObject::String("gpt-4o".into()),         // position 2: model
            ],
            &[],
            &(Arc::clone(&llm) as Arc<dyn crate::traits::llm::LlmBackend>),
            &mut tokens,
        )
        .await;

        match result {
            ExtFunctionResult::Return(MontyObject::List(items)) => assert_eq!(items.len(), 2),
            other => panic!("expected list return, got {other:?}"),
        }

        let calls = llm.calls.lock().await;
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|(m, _)| m.as_deref() == Some("gpt-4o")));
    }

    #[tokio::test]
    async fn llm_query_batched_honors_positional_models_list() {
        let llm = Arc::new(CapturingLlm::new());
        let mut tokens = crate::types::step::TokenUsage::default();
        let result = handle_llm_query_batched(
            &[
                MontyObject::List(vec![
                    MontyObject::String("q".into()),
                    MontyObject::String("q".into()),
                ]),
                MontyObject::None, // position 1: context = None
                MontyObject::None, // position 2: model = None
                MontyObject::List(vec![
                    // position 3: models
                    MontyObject::String("gpt-4o".into()),
                    MontyObject::String("claude-sonnet-4-6".into()),
                ]),
            ],
            &[],
            &(Arc::clone(&llm) as Arc<dyn crate::traits::llm::LlmBackend>),
            &mut tokens,
        )
        .await;

        assert!(matches!(result, ExtFunctionResult::Return(_)));
        let mut calls = llm.calls.lock().await;
        calls.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(calls[1].0.as_deref(), Some("gpt-4o"));
    }

    #[tokio::test]
    async fn llm_query_batched_positional_none_for_models_is_no_override() {
        // `llm_query_batched(prompts, None, None, None)` should run with no
        // model overrides, not error on the positional None at slot 3.
        let llm = Arc::new(CapturingLlm::new());
        let mut tokens = crate::types::step::TokenUsage::default();
        let result = handle_llm_query_batched(
            &[
                MontyObject::List(vec![MontyObject::String("a".into())]),
                MontyObject::None,
                MontyObject::None,
                MontyObject::None,
            ],
            &[],
            &(Arc::clone(&llm) as Arc<dyn crate::traits::llm::LlmBackend>),
            &mut tokens,
        )
        .await;

        assert!(matches!(result, ExtFunctionResult::Return(_)));
        let calls = llm.calls.lock().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, None);
    }

    #[tokio::test]
    async fn llm_query_batched_rejects_non_string_single_model() {
        let llm = Arc::new(CapturingLlm::new());
        let mut tokens = crate::types::step::TokenUsage::default();
        let prompts = MontyObject::List(vec![MontyObject::String("a".into())]);
        let result = handle_llm_query_batched(
            &[prompts],
            &[(MontyObject::String("model".into()), MontyObject::Int(7))],
            &(Arc::clone(&llm) as Arc<dyn crate::traits::llm::LlmBackend>),
            &mut tokens,
        )
        .await;

        assert!(matches!(result, ExtFunctionResult::Error(_)));
        assert!(llm.calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn llm_query_batched_rejects_non_string_models_entries() {
        let llm = Arc::new(CapturingLlm::new());
        let mut tokens = crate::types::step::TokenUsage::default();
        let prompts = MontyObject::List(vec![
            MontyObject::String("a".into()),
            MontyObject::String("b".into()),
        ]);
        // Integers in the models list should fail loudly, not be coerced to "1"/"2".
        let models = MontyObject::List(vec![MontyObject::Int(1), MontyObject::Int(2)]);
        let result = handle_llm_query_batched(
            &[prompts],
            &[(MontyObject::String("models".into()), models)],
            &(Arc::clone(&llm) as Arc<dyn crate::traits::llm::LlmBackend>),
            &mut tokens,
        )
        .await;

        assert!(matches!(result, ExtFunctionResult::Error(_)));
        assert!(llm.calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn llm_query_batched_none_in_models_list_does_not_backfill_from_model_kwarg() {
        // Regression: when `models=[None, "gpt-4o"]` and `model="claude-..."`
        // are both passed, the None slot must NOT be backfilled by the
        // singular `model=` kwarg. Each slot is authoritative.
        let llm = Arc::new(CapturingLlm::new());
        let mut tokens = crate::types::step::TokenUsage::default();
        let prompts = MontyObject::List(vec![
            MontyObject::String("a".into()),
            MontyObject::String("b".into()),
        ]);
        let models = MontyObject::List(vec![
            MontyObject::None,
            MontyObject::String("gpt-4o".into()),
        ]);
        let _ = handle_llm_query_batched(
            &[prompts],
            &[
                (MontyObject::String("models".into()), models),
                (
                    MontyObject::String("model".into()),
                    MontyObject::String("claude-sonnet-4-20250514".into()),
                ),
            ],
            &(Arc::clone(&llm) as Arc<dyn crate::traits::llm::LlmBackend>),
            &mut tokens,
        )
        .await;

        let calls = llm.calls.lock().await;
        assert_eq!(calls.len(), 2);
        // Slot 0 was None — must remain None, not become "claude-sonnet-4-20250514".
        let slot_a = calls.iter().find(|(_, p)| p == "a").expect("call for a");
        let slot_b = calls.iter().find(|(_, p)| p == "b").expect("call for b");
        assert_eq!(slot_a.0, None);
        assert_eq!(slot_b.0.as_deref(), Some("gpt-4o"));
    }

    #[tokio::test]
    async fn llm_query_batched_models_length_mismatch_errors() {
        let llm = Arc::new(CapturingLlm::new());
        let mut tokens = crate::types::step::TokenUsage::default();
        let prompts = MontyObject::List(vec![
            MontyObject::String("a".into()),
            MontyObject::String("b".into()),
        ]);
        let models = MontyObject::List(vec![MontyObject::String("only-one".into())]);
        let result = handle_llm_query_batched(
            &[prompts],
            &[(MontyObject::String("models".into()), models)],
            &(Arc::clone(&llm) as Arc<dyn crate::traits::llm::LlmBackend>),
            &mut tokens,
        )
        .await;

        assert!(matches!(result, ExtFunctionResult::Error(_)));
        assert!(llm.calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn execute_code_resolves_tool_info_schema_from_action_inventory_snapshot() {
        let thread = make_test_thread();
        let effects: Arc<dyn EffectExecutor> = Arc::new(SnapshotAwareToolInfoEffects);
        let leases = LeaseManager::new();
        let policy = PolicyEngine::new();
        let ctx = make_exec_context(&thread);

        leases
            .grant(thread.id, "tools", GrantedActions::All, None, None)
            .await
            .unwrap();

        let result = execute_code(
            r#"
result = await tool_info(name="mission-create", detail="schema")
"#,
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

        assert!(
            result.failure.is_none(),
            "unexpected failure: {:?}",
            result.failure
        );
        assert_eq!(result.action_results.len(), 1);
        assert!(!result.action_results[0].is_error);
        assert_eq!(
            result.action_results[0].output["name"],
            serde_json::json!("mission_create")
        );
        assert_eq!(
            result.action_results[0].output["schema"]["required"],
            serde_json::json!(["name", "goal", "cadence"])
        );
    }

    // ── Inline gate-await tests ─────────────────────────────────

    /// Effects stub: returns `Err(EngineError::GatePaused)` on first
    /// call for the given action, then a success result. Mimics a tool
    /// that gates mid-execution (e.g. policy escalation, leak detection).
    struct GatingThenOkEffects {
        action: String,
        success_output: serde_json::Value,
        call_count: Mutex<u32>,
        /// Captures `context.call_approval_granted` for every call.
        /// Lets tests assert the inline-retry path correctly forwards
        /// the user's one-shot approval to the host's
        /// `EffectExecutor` — the integration point where bugs in
        /// approval propagation surface.
        approval_flags_observed: Mutex<Vec<bool>>,
        actions: Vec<ActionDef>,
    }

    impl GatingThenOkEffects {
        fn new(action: &str, output: serde_json::Value) -> Self {
            Self {
                action: action.into(),
                success_output: output,
                call_count: Mutex::new(0),
                approval_flags_observed: Mutex::new(Vec::new()),
                actions: vec![test_action(action)],
            }
        }
        fn calls(&self) -> u32 {
            *self.call_count.lock().unwrap()
        }
        fn approval_flags(&self) -> Vec<bool> {
            self.approval_flags_observed.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl EffectExecutor for GatingThenOkEffects {
        async fn execute_action(
            &self,
            name: &str,
            params: serde_json::Value,
            _lease: &CapabilityLease,
            ctx: &ThreadExecutionContext,
        ) -> Result<ActionResult, EngineError> {
            let mut count = self.call_count.lock().unwrap();
            *count += 1;
            self.approval_flags_observed
                .lock()
                .unwrap()
                .push(ctx.call_approval_granted);
            if *count == 1 && name == self.action {
                return Err(EngineError::GatePaused {
                    gate_name: "approval".into(),
                    action_name: name.into(),
                    call_id: "test_call".into(),
                    parameters: Box::new(params),
                    resume_kind: Box::new(crate::gate::ResumeKind::Approval { allow_always: true }),
                    resume_output: None,
                    paused_lease: None,
                });
            }
            Ok(ActionResult {
                call_id: String::new(),
                action_name: name.into(),
                output: self.success_output.clone(),
                is_error: false,
                duration: Duration::from_millis(1),
            })
        }

        async fn available_actions(
            &self,
            _: &[CapabilityLease],
            _: &ThreadExecutionContext,
        ) -> Result<Vec<ActionDef>, EngineError> {
            Ok(self.actions.clone())
        }

        async fn available_capabilities(
            &self,
            _: &[CapabilityLease],
            _: &ThreadExecutionContext,
        ) -> Result<Vec<crate::types::capability::CapabilitySummary>, EngineError> {
            Ok(vec![])
        }
    }

    /// Test gate controller. Returns the canned resolution and records
    /// every pause request for assertion.
    struct StubGateController {
        resolution: Mutex<Option<crate::gate::GateResolution>>,
        pauses: Mutex<Vec<crate::gate::GatePauseRequest>>,
    }

    impl StubGateController {
        fn approving() -> Arc<Self> {
            Arc::new(Self {
                resolution: Mutex::new(Some(crate::gate::GateResolution::Approved {
                    always: false,
                })),
                pauses: Mutex::new(Vec::new()),
            })
        }
        fn denying() -> Arc<Self> {
            Arc::new(Self {
                resolution: Mutex::new(Some(crate::gate::GateResolution::Denied {
                    reason: Some("not now".into()),
                })),
                pauses: Mutex::new(Vec::new()),
            })
        }
        fn pause_count(&self) -> usize {
            self.pauses.lock().unwrap().len()
        }
    }

    #[async_trait::async_trait]
    impl crate::gate::GateController for StubGateController {
        async fn pause(
            &self,
            request: crate::gate::GatePauseRequest,
        ) -> crate::gate::GateResolution {
            self.pauses.lock().unwrap().push(request);
            self.resolution
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(crate::gate::GateResolution::Cancelled)
        }
    }

    /// Regression for the user-reported bug: a CodeAct script that
    /// calls a tool which gates mid-execution should NOT abort with
    /// `RuntimeError: execution paused by gate 'approval'`. With a
    /// controller wired and an `Approved` resolution, the script runs
    /// to completion and the tool's success result is delivered to
    /// Python.
    #[tokio::test]
    async fn codeact_gate_inline_await_approved_delivers_result() {
        let thread = make_test_thread();
        let effects = Arc::new(GatingThenOkEffects::new(
            "github_search",
            serde_json::json!({"items": [{"number": 1, "title": "ok"}]}),
        ));
        let effects_dyn: Arc<dyn EffectExecutor> = effects.clone();
        let controller = StubGateController::approving();
        let leases = LeaseManager::new();
        let policy = PolicyEngine::new();
        let mut ctx = make_exec_context(&thread);
        ctx.gate_controller = controller.clone() as Arc<dyn crate::gate::GateController>;

        leases
            .grant(thread.id, "tools", GrantedActions::All, None, None)
            .await
            .unwrap();

        let result = execute_code(
            r#"
result = await github_search(query="repo:foo")
print(result)
"#,
            &thread,
            &(Arc::new(StubLlm) as Arc<dyn crate::traits::llm::LlmBackend>),
            &effects_dyn,
            &leases,
            &policy,
            &ctx,
            &[],
            &serde_json::json!({}),
        )
        .await
        .expect("execute_code did not return Err");

        assert!(
            result.failure.is_none(),
            "expected clean completion, got failure: {:?}, stdout={}",
            result.failure,
            result.stdout
        );
        assert_eq!(controller.pause_count(), 1, "controller should pause once");
        assert!(
            !result.stdout.contains("execution paused by gate"),
            "stdout must not contain the pre-fix error string; got: {}",
            result.stdout
        );
        assert_eq!(
            effects.calls(),
            2,
            "expected one gating call + one retry after approval"
        );
        // The retry call MUST observe `call_approval_granted=true` so
        // the host's `EffectExecutor` skips its per-call approval
        // check. This is the regression coverage for serrrfirat's
        // review on PR #3157: without one-shot propagation, tools
        // with `ApprovalRequirement::Always` would gate again on
        // retry and the approval loop would never converge.
        assert_eq!(
            effects.approval_flags(),
            vec![false, true],
            "first call must be unapproved, retry must carry the user's approval"
        );
        assert_eq!(result.action_results.len(), 1);
        assert!(!result.action_results[0].is_error);
        assert_eq!(
            result.action_results[0].output["items"][0]["number"],
            serde_json::json!(1)
        );
    }

    /// Denial surfaces inside the script as a typed `RuntimeError`
    /// with a clear message — catchable by user code.
    #[tokio::test]
    async fn codeact_gate_inline_await_denied_raises_in_script() {
        let thread = make_test_thread();
        let effects = Arc::new(GatingThenOkEffects::new(
            "github_create_issue",
            serde_json::json!({"unused": true}),
        ));
        let effects_dyn: Arc<dyn EffectExecutor> = effects.clone();
        let controller = StubGateController::denying();
        let leases = LeaseManager::new();
        let policy = PolicyEngine::new();
        let mut ctx = make_exec_context(&thread);
        ctx.gate_controller = controller.clone() as Arc<dyn crate::gate::GateController>;

        leases
            .grant(thread.id, "tools", GrantedActions::All, None, None)
            .await
            .unwrap();

        // Uncaught: the script raises; the failure is observable on
        // CodeExecutionResult and the message identifies the tool +
        // reason so the caller / LLM can decide what to do next.
        let result = execute_code(
            r#"
result = await github_create_issue(title="x")
print(result)
"#,
            &thread,
            &(Arc::new(StubLlm) as Arc<dyn crate::traits::llm::LlmBackend>),
            &effects_dyn,
            &leases,
            &policy,
            &ctx,
            &[],
            &serde_json::json!({}),
        )
        .await
        .expect("execute_code did not return Err");

        assert_eq!(controller.pause_count(), 1);
        // The retry never happens on denial — only the initial call.
        assert_eq!(effects.calls(), 1, "denial must not retry the action");
        assert_eq!(
            result.failure,
            Some(CodeExecutionFailure::RuntimeError),
            "denial must surface as RuntimeError on CodeExecutionResult"
        );
        // The stdout message must identify the tool and the reason so
        // the caller can debug / surface to the LLM.
        assert!(
            result
                .stdout
                .contains("user denied tool 'github_create_issue'"),
            "denial message must identify the tool; got stdout: {}",
            result.stdout
        );
        assert!(
            result.stdout.contains("not now"),
            "denial message must include the user-supplied reason; got stdout: {}",
            result.stdout
        );
        // The pre-fix message must NOT appear — that was the user's
        // reported bug ("Error: RuntimeError: execution paused by
        // gate 'approval'").
        assert!(
            !result.stdout.contains("execution paused by gate"),
            "denial must not surface as the pre-fix bug message; got: {}",
            result.stdout
        );
    }

    /// With the default `CancellingGateController`, an `Approval` gate
    /// raised mid-execution surfaces as a typed cancellation that the
    /// script can catch as `RuntimeError`. The message must read
    /// "approval … unavailable" rather than "user denied" — the user
    /// never saw a prompt, so the deny framing would be misleading.
    ///
    /// Two regression checks in one test:
    /// - the pre-fix bug message (`"execution paused by gate 'approval'"`)
    ///   must NEVER appear — removing the inline-await wiring would
    ///   otherwise silently reproduce the original bug.
    /// - the message must NOT misattribute the cancellation to a user
    ///   denial; a future change that broadens `DenialOutcome::DeniedByUser`
    ///   to cover `Cancelled` would regress to the misleading wording.
    #[tokio::test]
    async fn codeact_default_controller_cancels_approval_gates() {
        let thread = make_test_thread();
        let effects = Arc::new(GatingThenOkEffects::new(
            "github_create_issue",
            serde_json::json!({"unused": true}),
        ));
        let effects_dyn: Arc<dyn EffectExecutor> = effects.clone();
        let leases = LeaseManager::new();
        let policy = PolicyEngine::new();
        // make_exec_context defaults to `CancellingGateController` —
        // this is the inert controller every test path uses, and
        // matches what production paths that don't pause supply.
        let ctx = make_exec_context(&thread);

        leases
            .grant(thread.id, "tools", GrantedActions::All, None, None)
            .await
            .unwrap();

        let result = execute_code(
            r#"
try:
    result = await github_create_issue(title="x")
    outcome = "approved"
except RuntimeError as e:
    outcome = str(e)
print(outcome)
"#,
            &thread,
            &(Arc::new(StubLlm) as Arc<dyn crate::traits::llm::LlmBackend>),
            &effects_dyn,
            &leases,
            &policy,
            &ctx,
            &[],
            &serde_json::json!({}),
        )
        .await
        .expect("execute_code did not return Err");

        assert!(
            !result.stdout.contains("execution paused by gate"),
            "the legacy bug message must never surface; got: {}",
            result.stdout
        );
        // CancellingGateController returns `Cancelled`, which now maps
        // to the `Unavailable` outcome — the script sees
        // "approval for tool '…' unavailable: approval cancelled".
        assert!(
            result
                .stdout
                .contains("approval for tool 'github_create_issue' unavailable"),
            "default controller must surface as approval-unavailable, not user-denied; got: {}",
            result.stdout
        );
        // Cancellation must NOT misattribute to a user denial — the
        // user never saw a prompt. A regression to "user denied tool
        // …: cancelled" would silently reintroduce the misleading
        // wording this DenialOutcome split exists to fix.
        assert!(
            !result.stdout.contains("user denied tool"),
            "cancellation must not surface as user-denied; got: {}",
            result.stdout
        );
        // Cancellation does not retry the action.
        assert_eq!(effects.calls(), 1);
    }

    // ── Error classification tests ──────────────────────────────

    #[test]
    fn classify_syntax_error() {
        let cat = classify_runtime_error("SyntaxError: unexpected token");
        assert_eq!(cat, CodeExecutionFailure::SyntaxError);
    }

    #[test]
    fn classify_timeout() {
        let cat = classify_runtime_error("execution timed out after 30s");
        assert_eq!(cat, CodeExecutionFailure::ResourceLimit);
    }

    #[test]
    fn classify_memory_limit() {
        let cat = classify_runtime_error("memory limit exceeded");
        assert_eq!(cat, CodeExecutionFailure::ResourceLimit);
    }

    #[test]
    fn classify_fuel_exhaustion() {
        let cat = classify_runtime_error("fuel exhausted during execution");
        assert_eq!(cat, CodeExecutionFailure::ResourceLimit);
    }

    #[test]
    fn classify_os_denied() {
        let cat = classify_runtime_error("OS operations are not permitted in CodeAct scripts");
        assert_eq!(cat, CodeExecutionFailure::OsDenied);
    }

    #[test]
    fn classify_name_error_as_runtime() {
        // NameError from Monty (not NameLookup) is classified as RuntimeError
        let cat = classify_runtime_error("NameError: name 'foo' is not defined");
        assert_eq!(cat, CodeExecutionFailure::RuntimeError);
    }

    #[test]
    fn classify_type_error_as_runtime() {
        let cat = classify_runtime_error("TypeError: unsupported operand");
        assert_eq!(cat, CodeExecutionFailure::RuntimeError);
    }

    #[test]
    fn classify_module_not_found_as_runtime() {
        let cat = classify_runtime_error("ModuleNotFoundError: No module named 'csv'");
        assert_eq!(cat, CodeExecutionFailure::RuntimeError);
    }

    #[test]
    fn classify_syntax_word_is_not_syntaxerror() {
        // "syntax" alone should not trigger SyntaxError — only "syntaxerror" should.
        let cat = classify_runtime_error("unexpected syntax in expression");
        assert_eq!(cat, CodeExecutionFailure::RuntimeError);
    }

    #[test]
    fn vm_panic_variant_serializes_as_snake_case() {
        // VmPanic is set directly by catch_unwind paths, not by classify_runtime_error.
        // Verify it serializes consistently with Display (both snake_case).
        let failure = CodeExecutionFailure::VmPanic;
        assert_eq!(failure.to_string(), "vm_panic");
        let json = serde_json::to_value(&failure).unwrap();
        assert_eq!(json, serde_json::json!("vm_panic"));
    }

    #[test]
    fn code_hash_deterministic() {
        let h1 = code_hash("print('hello')");
        let h2 = code_hash("print('hello')");
        assert_eq!(h1, h2);
    }

    #[test]
    fn code_hash_differs_for_different_code() {
        let h1 = code_hash("print('hello')");
        let h2 = code_hash("print('world')");
        assert_ne!(h1, h2);
    }
}
