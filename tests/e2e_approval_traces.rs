//! Replay coverage for the v1 approval round-trip.
//!
//! Phase 2 of #2828 — these are the first fixture-driven tests that
//! exercise the **full** approval cycle (pause → user resolution →
//! resume) rather than only the pause invariant. Companion file:
//! `tests/e2e_response_order.rs::no_done_emitted_while_awaiting_approval`,
//! which covers the pause but not the resume.
//!
//! Three scenarios:
//! - `approval_yes`: approve once → tool runs → final response
//! - `approval_no`: deny once → tool does not run → final response
//! - `approval_always`: allow-always on call 1 → call 2 runs without
//!   re-prompting

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod approval_trace_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;
    use ironclaw::channels::StatusUpdate;
    use ironclaw::context::JobContext;
    use ironclaw::tools::{ApprovalRequirement, Tool, ToolError, ToolOutput};

    const TIMEOUT: Duration = Duration::from_secs(15);

    /// Test tool whose approval requirement is `UnlessAutoApproved`. With
    /// `with_auto_approve_tools(false)` the agent must pause for user
    /// approval; an `always`-approve response should persist for the
    /// remainder of the session and skip the pause on subsequent calls.
    struct NeedsApprovalProbe {
        executions: Arc<AtomicUsize>,
    }

    impl NeedsApprovalProbe {
        fn new() -> (Arc<Self>, Arc<AtomicUsize>) {
            let executions = Arc::new(AtomicUsize::new(0));
            let tool = Arc::new(Self {
                executions: executions.clone(),
            });
            (tool, executions)
        }
    }

    #[async_trait]
    impl Tool for NeedsApprovalProbe {
        fn name(&self) -> &str {
            "needs_approval_probe"
        }

        fn description(&self) -> &str {
            "Test tool that requires approval unless auto-approved"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": { "value": { "type": "string" } },
                "required": ["value"]
            })
        }

        async fn execute(
            &self,
            params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            self.executions.fetch_add(1, Ordering::SeqCst);
            Ok(ToolOutput::success(
                serde_json::json!({"ok": true, "echoed": params}),
                Duration::from_millis(1),
            ))
        }

        fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
            ApprovalRequirement::UnlessAutoApproved
        }
    }

    /// Poll `captured_status_events` until an `ApprovalNeeded` is observed
    /// or the deadline elapses. Returns true on success.
    async fn wait_for_approval_needed(
        rig: &crate::support::test_rig::TestRig,
        timeout: Duration,
    ) -> bool {
        let initial = rig
            .captured_status_events()
            .iter()
            .filter(|s| matches!(s, StatusUpdate::ApprovalNeeded { .. }))
            .count();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let count = rig
                .captured_status_events()
                .iter()
                .filter(|s| matches!(s, StatusUpdate::ApprovalNeeded { .. }))
                .count();
            if count > initial {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn fixture_path(name: &str) -> String {
        format!(
            "{}/tests/fixtures/llm_traces/coverage/{}",
            env!("CARGO_MANIFEST_DIR"),
            name
        )
    }

    #[tokio::test]
    async fn approval_yes_runs_tool_and_produces_final_response() {
        let trace = LlmTrace::from_file(fixture_path("approval_yes.json"))
            .expect("failed to load approval_yes.json");
        let (tool, executions) = NeedsApprovalProbe::new();

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_extra_tools(vec![tool as Arc<dyn Tool>])
            .with_auto_approve_tools(false)
            .build()
            .await;
        rig.clear().await;

        rig.send_message("Run the gated probe").await;

        assert!(
            wait_for_approval_needed(&rig, TIMEOUT).await,
            "expected ApprovalNeeded status before approval was sent"
        );
        assert_eq!(
            executions.load(Ordering::SeqCst),
            0,
            "tool must not run before approval"
        );

        rig.send_message("yes").await;

        let responses = rig.wait_for_responses(1, TIMEOUT).await;
        assert_eq!(
            executions.load(Ordering::SeqCst),
            1,
            "tool must run exactly once after approval"
        );
        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }

    /// On deny, the agent does **not** call the LLM again — it surfaces a
    /// built-in rejection message directly to the user. The trace therefore
    /// only needs the initial tool-call step; the rejection text comes from
    /// the agent, not from a replayed LLM response.
    #[tokio::test]
    async fn approval_no_skips_tool_and_produces_final_response() {
        let trace = LlmTrace::from_file(fixture_path("approval_no.json"))
            .expect("failed to load approval_no.json");
        let (tool, executions) = NeedsApprovalProbe::new();

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_extra_tools(vec![tool as Arc<dyn Tool>])
            .with_auto_approve_tools(false)
            .build()
            .await;
        rig.clear().await;

        rig.send_message("Run the gated probe").await;

        assert!(
            wait_for_approval_needed(&rig, TIMEOUT).await,
            "expected ApprovalNeeded status before denial was sent"
        );

        rig.send_message("no").await;

        let responses = rig.wait_for_responses(1, TIMEOUT).await;
        assert_eq!(
            executions.load(Ordering::SeqCst),
            0,
            "tool must not run when approval is denied"
        );
        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }

    #[tokio::test]
    async fn approval_always_persists_for_subsequent_calls() {
        let trace = LlmTrace::from_file(fixture_path("approval_always.json"))
            .expect("failed to load approval_always.json");
        let (tool, executions) = NeedsApprovalProbe::new();

        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_extra_tools(vec![tool as Arc<dyn Tool>])
            .with_auto_approve_tools(false)
            .build()
            .await;
        rig.clear().await;

        rig.send_message("Run the gated probe twice").await;

        assert!(
            wait_for_approval_needed(&rig, TIMEOUT).await,
            "expected ApprovalNeeded status before allow-always was sent"
        );
        let approval_needed_after_first = rig
            .captured_status_events()
            .iter()
            .filter(|s| matches!(s, StatusUpdate::ApprovalNeeded { .. }))
            .count();
        assert_eq!(
            approval_needed_after_first, 1,
            "exactly one ApprovalNeeded should be pending before resolving"
        );

        rig.send_message("always").await;

        let responses = rig.wait_for_responses(1, TIMEOUT).await;
        assert_eq!(
            executions.load(Ordering::SeqCst),
            2,
            "both tool calls must run after allow-always"
        );

        let total_approval_needed = rig
            .captured_status_events()
            .iter()
            .filter(|s| matches!(s, StatusUpdate::ApprovalNeeded { .. }))
            .count();
        assert_eq!(
            total_approval_needed, 1,
            "second tool call must not re-prompt for approval after allow-always; got {} prompts",
            total_approval_needed
        );

        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }
}
