//! Replay coverage for the engine-v2 authentication gate round-trip.
//!
//! Phase 2 of #2828 — extends the approval fixtures with the typed
//! `Submission::GateAuthResolution` / `Submission::ExternalCallback`
//! path that only engine v2 recognizes. The tests drive the full
//! cycle: LLM → `tool_activate` → GatePaused(Authentication) →
//! typed resolution → re-run → final response.
//!
//! Scenarios:
//! - `auth_credential_provided_resumes_action` — happy path
//! - `auth_cancelled_stops_thread` — cancel path
//! - `auth_retry_after_invalid_credential` — second pause gets a new request_id
//! - `auth_external_callback_resolves_oauth_gate` — OAuth callback path
//! - `auth_gate_emits_request_id_for_v2` — invariant: gate carries a Uuid

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod auth_gate_trace_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, OnceLock};
    use std::time::Duration;

    use async_trait::async_trait;
    use tokio::sync::Mutex;

    use crate::support::test_rig::{TestRig, TestRigBuilder};
    use crate::support::trace_llm::LlmTrace;
    use ironclaw::agent::submission::AuthGateResolution;
    use ironclaw::channels::StatusUpdate;
    use ironclaw::context::JobContext;
    use ironclaw::tools::{Tool, ToolError, ToolOutput};

    const TIMEOUT: Duration = Duration::from_secs(15);

    /// Serialize all tests in this file because engine v2 stores its state
    /// in a process-global `OnceLock<RwLock<Option<EngineState>>>`.
    /// Running these tests in parallel would let one test's gate_paused
    /// state leak into the next test's engine instance.
    fn engine_v2_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    /// Test stub for the `tool_activate` built-in. Returns a pre-configured
    /// sequence of JSON outputs so the test can shape the GatePaused path.
    ///
    /// Each call pops the next output off the queue; after the queue is
    /// drained the last output is reused (so resumes after the final
    /// gate always see the "ready" state).
    struct MockActivateTool {
        outputs: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
        executions: Arc<AtomicUsize>,
    }

    impl MockActivateTool {
        fn new(outputs: Vec<serde_json::Value>) -> (Arc<Self>, Arc<AtomicUsize>) {
            let executions = Arc::new(AtomicUsize::new(0));
            let tool = Arc::new(Self {
                outputs: Arc::new(std::sync::Mutex::new(outputs)),
                executions: executions.clone(),
            });
            (tool, executions)
        }
    }

    #[async_trait]
    impl Tool for MockActivateTool {
        fn name(&self) -> &str {
            // Must match the protected built-in name so the effect adapter's
            // `auth_gate_from_extension_result` path inspects our output.
            "tool_activate"
        }

        fn description(&self) -> &str {
            "Test stub for tool_activate that emits scripted auth-gate outputs"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "token": { "type": "string" }
                },
                "required": ["name"]
            })
        }

        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            self.executions.fetch_add(1, Ordering::SeqCst);
            let mut q = self.outputs.lock().expect("outputs lock poisoned");
            let next = if q.len() > 1 {
                q.remove(0)
            } else {
                // Last output — reuse on every subsequent call so the
                // post-credential re-run returns the "ready" state.
                q.first().cloned().unwrap_or(serde_json::json!({}))
            };
            Ok(ToolOutput::success(next, Duration::from_millis(1)))
        }
    }

    /// Poll `captured_status_events` until an `AuthRequired` with a
    /// `request_id` is observed (past `initial_count`). Returns the first
    /// new `request_id` as a `Uuid`, or a descriptive error when the
    /// router emitted a malformed ID or the status never arrived.
    async fn wait_for_auth_required(
        rig: &TestRig,
        initial_count: usize,
        timeout: Duration,
    ) -> Result<uuid::Uuid, String> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let events = rig.captured_status_events();
            let with_id: Vec<_> = events
                .iter()
                .filter_map(|s| match s {
                    StatusUpdate::AuthRequired {
                        request_id: Some(id),
                        ..
                    } => Some(id.clone()),
                    _ => None,
                })
                .collect();
            if with_id.len() > initial_count {
                let raw = with_id
                    .get(initial_count)
                    .expect("length checked above; request_id must exist");
                return uuid::Uuid::parse_str(raw)
                    .map_err(|e| format!("malformed AuthRequired.request_id {raw:?}: {e}"));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "timed out waiting for AuthRequired #{initial_count}; saw events: {events:?}"
                ));
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

    /// Write a minimal SKILL.md that declares a credential spec for
    /// `credential_name` into `skills_dir`. This is the bare-minimum fixture
    /// that makes `AuthManager::submit_auth_token` accept the credential
    /// (it requires a matching skill credential spec when no extension is
    /// installed). The skill body is intentionally trivial — the test does
    /// not exercise prompt/activation behavior.
    fn plant_probe_skill(skills_dir: &std::path::Path, credential_name: &str) {
        let skill_dir = skills_dir.join(format!("probe_{credential_name}"));
        std::fs::create_dir_all(&skill_dir).expect("create probe skill dir");
        let manifest = format!(
            r#"---
name: probe_{credential_name}
version: "0.0.0"
description: Probe skill for auth-gate replay coverage.
activation:
  keywords:
    - "probe-skill-should-never-activate-in-this-test"
credentials:
  - name: {credential_name}
    provider: test
    location:
      type: bearer
    hosts:
      - "example.com"
---

Probe skill.
"#
        );
        std::fs::write(skill_dir.join("SKILL.md"), manifest).expect("write probe SKILL.md");
    }

    /// Common rig setup: engine v2 + a `MockActivateTool` override keyed by
    /// the caller's scripted output sequence. Also plants a probe skill for
    /// each credential name referenced in `outputs` so the auth-manager
    /// credential-store path accepts the token during `CredentialProvided`.
    async fn auth_rig(
        trace: LlmTrace,
        outputs: Vec<serde_json::Value>,
    ) -> (TestRig, Arc<AtomicUsize>) {
        // Extract credential names so we can plant matching probe skills.
        let credential_names: std::collections::HashSet<String> = outputs
            .iter()
            .filter_map(|v| v.get("name").and_then(|n| n.as_str()).map(String::from))
            .collect();

        // Use `.into_path()` to release the tempdir from auto-cleanup so
        // skill discovery can read from it after the builder finishes. OS
        // cleanup handles the directory between runs.
        let skills_tempdir = tempfile::tempdir().expect("probe skills tempdir");
        let skills_path = skills_tempdir.keep();
        for name in &credential_names {
            plant_probe_skill(&skills_path, name);
        }

        let (tool, executions) = MockActivateTool::new(outputs);
        let rig = TestRigBuilder::new()
            .with_trace(trace)
            .with_engine_v2()
            .with_test_tool_override(tool as Arc<dyn Tool>)
            .with_skills_dir(skills_path)
            .build()
            .await;
        rig.clear().await;
        (rig, executions)
    }

    #[tokio::test]
    async fn auth_credential_provided_resumes_action() {
        let _guard = engine_v2_test_lock().lock().await;
        let trace = LlmTrace::from_file(fixture_path("auth_credential_provided.json"))
            .expect("failed to load auth_credential_provided.json");
        let outputs = vec![
            serde_json::json!({
                "status": "awaiting_token",
                "name": "test_credential",
                "instructions": "Provide test token"
            }),
            serde_json::json!({
                "status": "ready",
                "name": "test_credential",
                "message": "Credential configured"
            }),
        ];
        let (rig, executions) = auth_rig(trace.clone(), outputs).await;

        rig.send_message("Set up the test credential").await;

        let request_id = wait_for_auth_required(&rig, 0, TIMEOUT)
            .await
            .expect("expected AuthRequired with request_id before token was provided");

        rig.send_gate_auth_resolution(
            request_id,
            AuthGateResolution::CredentialProvided {
                token: "test-token-value".to_string(),
            },
        )
        .await;

        let responses = rig.wait_for_responses(1, TIMEOUT).await;
        assert!(
            executions.load(Ordering::SeqCst) >= 2,
            "tool must re-run after credential submission (executions={})",
            executions.load(Ordering::SeqCst)
        );
        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }

    #[tokio::test]
    async fn auth_cancelled_stops_thread() {
        let _guard = engine_v2_test_lock().lock().await;
        let trace = LlmTrace::from_file(fixture_path("auth_cancelled.json"))
            .expect("failed to load auth_cancelled.json");
        let outputs = vec![serde_json::json!({
            "status": "awaiting_token",
            "name": "test_credential",
            "instructions": "Provide test token"
        })];
        let (rig, executions) = auth_rig(trace.clone(), outputs).await;

        rig.send_message("Set up the credential I'll cancel").await;

        let request_id = wait_for_auth_required(&rig, 0, TIMEOUT)
            .await
            .expect("expected AuthRequired before cancel");

        rig.send_gate_auth_resolution(request_id, AuthGateResolution::Cancelled)
            .await;

        let responses = rig.wait_for_responses(1, TIMEOUT).await;
        assert_eq!(
            executions.load(Ordering::SeqCst),
            1,
            "tool must run exactly once (cancel must not trigger a re-run)"
        );
        assert!(
            responses.iter().any(|r| r.content.contains("Cancelled")),
            "expected 'Cancelled.' response after auth-gate cancel, got: {:?}",
            responses.iter().map(|r| &r.content).collect::<Vec<_>>()
        );
        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }

    #[tokio::test]
    async fn auth_retry_after_invalid_credential() {
        let _guard = engine_v2_test_lock().lock().await;
        let trace = LlmTrace::from_file(fixture_path("auth_retry_invalid_then_valid.json"))
            .expect("failed to load auth_retry_invalid_then_valid.json");
        // First two calls pause the gate (first token is "invalid"), third
        // call returns ready. The mock doesn't actually validate tokens —
        // it just emits the scripted output sequence.
        let outputs = vec![
            serde_json::json!({
                "status": "awaiting_token",
                "name": "test_credential",
                "instructions": "Provide test token"
            }),
            serde_json::json!({
                "status": "awaiting_token",
                "name": "test_credential",
                "instructions": "That token was invalid; try again"
            }),
            serde_json::json!({
                "status": "ready",
                "name": "test_credential"
            }),
        ];
        let (rig, executions) = auth_rig(trace.clone(), outputs).await;

        rig.send_message("Set up the credential").await;

        let first_id = wait_for_auth_required(&rig, 0, TIMEOUT)
            .await
            .expect("expected first AuthRequired");
        rig.send_gate_auth_resolution(
            first_id,
            AuthGateResolution::CredentialProvided {
                token: "invalid".to_string(),
            },
        )
        .await;

        let second_id = wait_for_auth_required(&rig, 1, TIMEOUT)
            .await
            .expect("expected second AuthRequired after invalid token");
        assert_ne!(
            first_id, second_id,
            "re-pause must emit a fresh request_id, not reuse the first"
        );

        rig.send_gate_auth_resolution(
            second_id,
            AuthGateResolution::CredentialProvided {
                token: "valid".to_string(),
            },
        )
        .await;

        let responses = rig.wait_for_responses(1, TIMEOUT).await;
        assert!(
            executions.load(Ordering::SeqCst) >= 3,
            "tool must run at least three times (initial + two resumes), got {}",
            executions.load(Ordering::SeqCst)
        );
        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }

    #[tokio::test]
    async fn auth_external_callback_resolves_oauth_gate() {
        let _guard = engine_v2_test_lock().lock().await;
        let trace = LlmTrace::from_file(fixture_path("auth_external_callback.json"))
            .expect("failed to load auth_external_callback.json");
        let outputs = vec![
            serde_json::json!({
                "status": "awaiting_authorization",
                "name": "oauth_service",
                "auth_url": "https://example.com/oauth/start",
                "instructions": "Complete the OAuth flow in your browser"
            }),
            serde_json::json!({
                "status": "ready",
                "name": "oauth_service"
            }),
        ];
        let (rig, executions) = auth_rig(trace.clone(), outputs).await;

        rig.send_message("Connect the OAuth service").await;

        let request_id = wait_for_auth_required(&rig, 0, TIMEOUT)
            .await
            .expect("expected AuthRequired with request_id for OAuth");

        // Confirm the gate carried an auth_url (OAuth shape, not bare token).
        assert!(
            rig.captured_status_events().iter().any(|s| matches!(
                s,
                StatusUpdate::AuthRequired { auth_url: Some(url), .. } if url.starts_with("https://example.com/oauth")
            )),
            "AuthRequired for OAuth must surface the auth_url"
        );

        rig.send_external_callback(request_id).await;

        let responses = rig.wait_for_responses(1, TIMEOUT).await;
        assert!(
            executions.load(Ordering::SeqCst) >= 2,
            "tool must re-run after external callback (executions={})",
            executions.load(Ordering::SeqCst)
        );
        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }

    #[tokio::test]
    async fn auth_gate_emits_request_id_for_v2() {
        let _guard = engine_v2_test_lock().lock().await;
        let trace = LlmTrace::from_file(fixture_path("auth_gate_request_id.json"))
            .expect("failed to load auth_gate_request_id.json");
        let outputs = vec![serde_json::json!({
            "status": "awaiting_token",
            "name": "probe_credential",
            "instructions": "Provide"
        })];
        let (rig, _executions) = auth_rig(trace, outputs).await;

        rig.send_message("Trigger the auth gate").await;

        let request_id = wait_for_auth_required(&rig, 0, TIMEOUT)
            .await
            .expect("engine v2 must always emit a request_id on AuthRequired");
        // Invariant: the gate's request_id must round-trip through Uuid parsing.
        // (`wait_for_auth_required` already parsed it; this asserts non-nil.)
        assert_ne!(
            request_id,
            uuid::Uuid::nil(),
            "request_id on AuthRequired must be a real Uuid, not nil"
        );

        // Clean up the pending gate so the test doesn't leak an active thread.
        rig.send_gate_auth_resolution(request_id, AuthGateResolution::Cancelled)
            .await;
        rig.shutdown();
    }
}
