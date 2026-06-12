//! Regression coverage for issue #2544 — "agent plans and delegates tasks
//! but never executes or completes them" — captured here as the broader
//! claim/evidence drift class documented in `.claude/rules/tool-evidence.md`
//! ("Engine v2 Side-Effect Gate (target invariant)").
//!
//! The gate that satisfies these tests lives in
//! `crates/ironclaw_engine/orchestrator/default.py`
//! (`_maybe_evidence_drift_error`). It fires at every `FINAL`
//! extraction point — text, code, and tool_calls — purely on the
//! thread's cumulative `(success_count, failure_count)` counters. No
//! classification of FINAL text or user messages.
//!
//! The structural rule:
//!
//!     reject iff failure_count > 0 AND success_count == 0
//!
//! That is: the thread attempted at least one tool, every attempt
//! failed, and the model is now trying to FINAL. The orchestrator
//! transitions the thread to `Failed` and surfaces "Action not
//! performed" rather than the agent's narration. The prompt-side
//! guidance in `crates/ironclaw_engine/prompts/codeact_postamble.md`
//! ("Claims in `FINAL()` need tool evidence") still applies — these
//! tests are the structural safety net for the model failing to
//! follow that guidance.
//!
//! Scenarios:
//!
//! 1. `multi_iteration_tool_failure_then_lying_final` — iter 1 emits Python
//!    that calls a failing tool (raises a `RuntimeError`), iter 2 emits
//!    `FINAL("Message sent successfully!")`. Mirrors the trace from #2544 /
//!    #2580: a tool fails, the model re-plans, and the next turn fabricates
//!    success. Counters: 1 failure, 0 successes → reject.
//!
//! 2. `single_script_swallows_failure_then_lying_final` — one iteration of
//!    Python where the tool returns a *soft* failure (`is_error: true`),
//!    the script discards the error value and falls through to a `FINAL`
//!    that asserts success. Counters: 1 failure, 0 successes → reject.
//!
//! 3. `no_await_tool_call_then_lying_final` — `#[ignore]`-marked. The
//!    script names the tool but omits `await`, so Monty returns a
//!    coroutine and nothing is dispatched. Counters: 0 failure, 0
//!    success → the structural rule cannot distinguish this from a
//!    legitimate informational answer. Preserved as a `#[ignore]`
//!    regression anchor for a future intent-aware or AST-aware gate.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ironclaw_engine::runtime::messaging::signal_channel;
use ironclaw_engine::{
    ActionDef, ActionInventory, ActionResult, CancellingGateController, CapabilityLease,
    CapabilitySummary, EffectExecutor, EffectType, EngineError, ExecutionLoop, GrantedActions,
    LeaseManager, LlmBackend, LlmCallConfig, LlmOutput, LlmResponse, ModelToolSurface,
    PolicyEngine, ProjectId, Thread, ThreadConfig, ThreadExecutionContext, ThreadMessage,
    ThreadOutcome, ThreadType, TokenUsage,
};

// ── Mocks ─────────────────────────────────────────────────────

/// LLM that replays a queued sequence of [`LlmOutput`] values.
struct QueueLlm {
    responses: Mutex<Vec<LlmOutput>>,
}

#[async_trait::async_trait]
impl LlmBackend for QueueLlm {
    async fn complete(
        &self,
        _messages: &[ThreadMessage],
        _actions: &[ActionDef],
        _config: &LlmCallConfig,
    ) -> Result<LlmOutput, EngineError> {
        let mut q = self.responses.lock().expect("QueueLlm mutex poisoned");
        if q.is_empty() {
            // Defensive: the test should be deterministic; if we ran out of
            // scripted responses, return text that the assertion can clearly
            // distinguish from any expected outcome.
            return Ok(LlmOutput {
                response: LlmResponse::Text("(no more scripted LLM responses)".into()),
                usage: TokenUsage::default(),
            });
        }
        Ok(q.remove(0))
    }

    fn model_name(&self) -> &str {
        "queue-llm"
    }
}

/// EffectExecutor that exposes a single `send_message` action and replays a
/// queued sequence of results — each invocation pops the next entry.
struct QueueEffects {
    actions: Vec<ActionDef>,
    results: Mutex<Vec<Result<ActionResult, EngineError>>>,
}

#[async_trait::async_trait]
impl EffectExecutor for QueueEffects {
    async fn execute_action(
        &self,
        _action_name: &str,
        _parameters: serde_json::Value,
        _lease: &CapabilityLease,
        _context: &ThreadExecutionContext,
    ) -> Result<ActionResult, EngineError> {
        let mut q = self.results.lock().expect("QueueEffects mutex poisoned");
        if q.is_empty() {
            return Err(EngineError::Effect {
                reason: "test bug: no more scripted tool results".into(),
            });
        }
        q.remove(0)
    }

    async fn available_actions(
        &self,
        _leases: &[CapabilityLease],
        _context: &ThreadExecutionContext,
    ) -> Result<Vec<ActionDef>, EngineError> {
        Ok(self.actions.clone())
    }

    async fn available_action_inventory(
        &self,
        _leases: &[CapabilityLease],
        _context: &ThreadExecutionContext,
    ) -> Result<ActionInventory, EngineError> {
        Ok(ActionInventory {
            inline: self.actions.clone(),
            discoverable: Vec::new(),
        })
    }

    async fn available_capabilities(
        &self,
        _leases: &[CapabilityLease],
        _context: &ThreadExecutionContext,
    ) -> Result<Vec<CapabilitySummary>, EngineError> {
        Ok(Vec::new())
    }
}

fn send_message_action() -> ActionDef {
    ActionDef {
        name: "send_message".into(),
        description: "Send a chat message to a recipient.".into(),
        parameters_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "to":   { "type": "string" },
                "text": { "type": "string" }
            }
        }),
        // `WriteExternal` so the action is unambiguously side-effecting —
        // the kind of effect the (future) side-effect gate would care about.
        effects: vec![EffectType::WriteExternal],
        requires_approval: false,
        model_tool_surface: ModelToolSurface::FullSchema,
        discovery: None,
    }
}

fn code_response(code: &str) -> LlmOutput {
    LlmOutput {
        response: LlmResponse::Code {
            code: code.into(),
            content: Some(format!("```repl\n{code}\n```")),
        },
        usage: TokenUsage {
            input_tokens: 100,
            output_tokens: 80,
            ..Default::default()
        },
    }
}

async fn build_loop(
    llm_responses: Vec<LlmOutput>,
    tool_results: Vec<Result<ActionResult, EngineError>>,
) -> ExecutionLoop {
    let project_id = ProjectId::new();
    let thread = Thread::new(
        "send a chat message to alice saying hello",
        ThreadType::Foreground,
        project_id,
        "test-user",
        ThreadConfig {
            // Bound the loop tightly so a buggy mock can't run away.
            max_iterations: 5,
            // The text-only nudge would never fire for Code responses, but
            // turning it off keeps the trace easier to read if a future
            // refactor changes that.
            enable_tool_intent_nudge: false,
            ..ThreadConfig::default()
        },
    );
    let tid = thread.id;

    let llm = Arc::new(QueueLlm {
        responses: Mutex::new(llm_responses),
    });
    let effects = Arc::new(QueueEffects {
        actions: vec![send_message_action()],
        results: Mutex::new(tool_results),
    });
    let leases = Arc::new(LeaseManager::new());
    let policy = Arc::new(PolicyEngine::new());

    leases
        .grant(tid, "messaging", GrantedActions::All, None, None)
        .await
        .expect("granting a wide-open lease must not fail");

    let (_tx, rx) = signal_channel(16);

    ExecutionLoop::new(
        thread,
        llm,
        effects,
        leases,
        policy,
        rx,
        "test-user".into(),
        CancellingGateController::arc(),
    )
}

/// The scripted lying FINAL that both failing-tool tests use. Kept as a
/// constant so the orchestrator-side gate and the test-side assertion
/// agree on exactly one string, with no NLP classification on either
/// side. Tests that change the scripted FINAL must update this too.
const LYING_FINAL: &str = "Message sent successfully!";

/// Assert the invariant the orchestrator's evidence-drift gate enforces:
/// the scripted lying FINAL must not surface verbatim through a
/// `Completed` outcome. Either:
///
/// * The engine rejects the unsupported claim (`ThreadOutcome::Failed`), or
/// * The response is something other than the scripted lie (e.g. a
///   sentinel like "(no more scripted LLM responses)" if the loop
///   continued past the lying iteration).
///
/// The gate fires in `crates/ironclaw_engine/orchestrator/default.py`
/// (`_maybe_evidence_drift_error`) and produces the `Failed` shape with
/// an "Action not performed" error message — see
/// `.claude/rules/tool-evidence.md` (issue #2544).
fn assert_no_unsupported_success_claim(outcome: &ThreadOutcome) {
    match outcome {
        ThreadOutcome::Completed { response: Some(r) } => {
            assert_ne!(
                r, LYING_FINAL,
                "ISSUE #2544 evidence drift: the scripted lying FINAL \
                 surfaced verbatim through a Completed outcome. The \
                 orchestrator's evidence-drift gate should have produced \
                 ThreadOutcome::Failed instead. See \
                 .claude/rules/tool-evidence.md."
            );
        }
        ThreadOutcome::Completed { response: None } => {
            // Acceptable: nothing was claimed.
        }
        ThreadOutcome::Failed { .. } => {
            // Acceptable: the gate rejected the unsupported claim. This is
            // one of the two desired shapes.
        }
        ThreadOutcome::Stopped
        | ThreadOutcome::MaxIterations
        | ThreadOutcome::GatePaused { .. } => {
            panic!(
                "expected Completed-with-honest-response or Failed, got {:?}",
                outcome
            );
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────

/// Multi-iteration scenario (primary regression case for #2544).
///
/// Iteration 1: the model emits Python that calls `send_message(...)`. The
/// tool returns `Err(EngineError::Effect{...})`, which the scripting layer
/// turns into a `RuntimeError` inside Monty — the script aborts and `FINAL`
/// is never extracted. Iteration 2: the model emits a fresh code block
/// that calls `FINAL("Message sent successfully!")` without re-trying the
/// failed action. The desired behavior is that the engine refuses to
/// surface that claim as a successful completion.
#[tokio::test]
async fn multi_iteration_tool_failure_then_lying_final() {
    let lying_code = format!(r#"FINAL("{LYING_FINAL}")"#);
    let mut exec = build_loop(
        vec![
            code_response(
                r#"
result = await send_message(to="alice", text="hello")
FINAL(f"Message sent: {result}")
"#,
            ),
            code_response(&lying_code),
        ],
        vec![Err(EngineError::Effect {
            reason: "transport closed by peer".into(),
        })],
    )
    .await;

    let outcome = exec.run().await.expect("execution loop should not error");

    // Sanity: the failing tool actually got dispatched in iteration 1, and
    // the loop reached iteration 2 where the lying FINAL is emitted. Both
    // are required for this scenario to exercise the drift path.
    let action_failures = exec
        .thread
        .events
        .iter()
        .filter(|e| {
            matches!(
                &e.kind,
                ironclaw_engine::EventKind::ActionFailed { action_name, .. }
                    if action_name == "send_message"
            )
        })
        .count();
    assert_eq!(
        action_failures, 1,
        "expected exactly one ActionFailed event for send_message, got {action_failures}"
    );
    assert!(
        exec.thread.step_count >= 2,
        "expected at least 2 steps, got {} — the second iteration is \
         where the lying FINAL is emitted",
        exec.thread.step_count
    );

    assert_no_unsupported_success_claim(&outcome);
}

/// Single-script scenario: one iteration where the tool returns a *soft*
/// error (`ActionResult { is_error: true, ... }`), the script ignores it,
/// and `FINAL(...)` claims success in the same step.
///
/// Soft error rather than `Err(EngineError::Effect{...})` because a
/// host-raised `RuntimeError` from the hard-error path escapes Python's
/// `try/except Exception` in Monty (it terminates the whole code step
/// and shows up as a `CodeExecutionFailed` event), so the original
/// try/except shape can't actually call `FINAL` in a single iteration
/// today. The soft path is the realistic one for #2544: the tool *did*
/// surface its failure (an `ActionFailed` event fires and the Python
/// value is an `{"error": ...}` dict) but the model swallows the
/// failure value and narrates a success it cannot back up.
///
/// Complements the multi-iteration test: the gate inspects cumulative
/// successful-action count, not "was *any* action attempted", so a step
/// whose only action failed cannot satisfy a `FINAL` that claims
/// success.
#[tokio::test]
async fn single_script_swallows_failure_then_lying_final() {
    let lying_code = format!(
        r#"
result = await send_message(to="alice", text="hello")
# The model "sees" the error in `result` but narrates success anyway.
FINAL("{LYING_FINAL}")
"#
    );
    let mut exec = build_loop(
        vec![code_response(&lying_code)],
        vec![Ok(ActionResult {
            call_id: "code_call_1".into(),
            action_name: "send_message".into(),
            output: serde_json::json!({"error": "transport closed by peer"}),
            is_error: true,
            duration: Duration::from_millis(3),
        })],
    )
    .await;

    let outcome = exec.run().await.expect("execution loop should not error");

    // Sanity: the soft-failing tool actually got dispatched and surfaced as
    // ActionFailed. Without this, a future change that breaks
    // `await tool(...)` dispatch would also satisfy "no successful
    // evidence" and the test would pass for the wrong reason.
    let action_failures = exec
        .thread
        .events
        .iter()
        .filter(|e| {
            matches!(
                &e.kind,
                ironclaw_engine::EventKind::ActionFailed { action_name, .. }
                    if action_name == "send_message"
            )
        })
        .count();
    assert_eq!(
        action_failures, 1,
        "expected exactly one ActionFailed event for send_message, got {action_failures}"
    );
    assert_eq!(
        exec.thread.step_count, 1,
        "single-script scenario must finish in exactly one step"
    );

    assert_no_unsupported_success_claim(&outcome);
}

/// No-`await` variant: the script *names* the tool but never awaits it,
/// so Monty returns a coroutine object instead of dispatching. The host
/// never sees an `ActionExecuted` or `ActionFailed` event, but `FINAL`
/// still claims success.
///
/// **Out of scope for the v1 structural gate.** The gate fires only
/// when there is at least one failed attempt and zero successes; here
/// there are zero attempts of either flavor (Monty produced a
/// coroutine, no host dispatch ever happened), so the gate cannot
/// distinguish "model lied about a tool that was never called" from
/// "model gave an informational answer that needed no tool". Catching
/// this requires either:
///
///   * intent classification on the user message (rejected — adds NLP
///     surface), or
///   * AST inspection of the emitted code to detect "names a tool but
///     never awaits it".
///
/// Kept as `#[ignore]` to preserve the bug shape as a regression
/// anchor. The test currently *fails* (the misleading FINAL is
/// surfaced as `Completed`), and that failure is the documentation of
/// the gap. Remove the `#[ignore]` once one of the strategies above
/// lands and re-tighten the assertion.
#[ignore = "known gap: no-await tools register as 0 attempts, not a failure (#2544 follow-up)"]
#[tokio::test]
async fn no_await_tool_call_then_lying_final() {
    let lying_code = format!(
        r#"
# Missing `await` — `send_message(...)` returns a coroutine object,
# the tool is never dispatched, and the host records no ActionExecuted
# or ActionFailed event. The model still claims success.
_unused = send_message(to="alice", text="hello")
FINAL("{LYING_FINAL}")
"#
    );
    let mut exec = build_loop(
        vec![code_response(&lying_code)],
        // Pre-loaded tool result that the scripting layer will not
        // consume because the tool is never dispatched. Present here so
        // the QueueEffects mock cannot pop an "out of results" error and
        // confuse the assertion.
        vec![Ok(ActionResult {
            call_id: "unused".into(),
            action_name: "send_message".into(),
            output: serde_json::json!({"message_id": "would-have-been-m1"}),
            is_error: false,
            duration: Duration::from_millis(1),
        })],
    )
    .await;

    let outcome = exec.run().await.expect("execution loop should not error");

    // Sanity: the tool was NOT dispatched. No ActionExecuted or
    // ActionFailed event mentions send_message. This is what
    // distinguishes the no-await case from the soft-error case: the
    // host has *zero* signal that anything was attempted.
    let send_events = exec
        .thread
        .events
        .iter()
        .filter(|e| {
            matches!(
                &e.kind,
                ironclaw_engine::EventKind::ActionExecuted { action_name, .. }
                | ironclaw_engine::EventKind::ActionFailed { action_name, .. }
                    if action_name == "send_message"
            )
        })
        .count();
    assert_eq!(
        send_events, 0,
        "expected zero ActionExecuted/ActionFailed events for send_message \
         (the missing `await` should leave the tool undispatched), got {send_events}"
    );

    assert_no_unsupported_success_claim(&outcome);
}

/// Sanity check: when the tool actually succeeds, the same `FINAL(...)`
/// claim is fine — the gate does not reject legitimate completions. The
/// `send_message` call returned a non-error `ActionResult` with a real
/// `message_id`, so cumulative successful-action count is non-zero and
/// the gate stands down. Locks the gate against over-broad rejection.
#[tokio::test]
async fn successful_tool_then_final_is_allowed() {
    let mut exec = build_loop(
        vec![code_response(
            r#"
result = await send_message(to="alice", text="hello")
FINAL(f"Sent (message_id={result['message_id']}).")
"#,
        )],
        vec![Ok(ActionResult {
            call_id: "test_call_1".into(),
            action_name: "send_message".into(),
            output: serde_json::json!({"message_id": "m_42"}),
            is_error: false,
            duration: Duration::from_millis(7),
        })],
    )
    .await;

    let outcome = exec.run().await.expect("execution loop should not error");

    match outcome {
        ThreadOutcome::Completed { response: Some(r) } => {
            // Real evidence — `message_id=m_42` — must round-trip into FINAL.
            assert!(
                r.contains("m_42"),
                "FINAL should carry the real message_id, got: {r:?}",
            );
        }
        other => panic!("expected Completed with response, got {other:?}"),
    }
}
