//! Dual-mode E2E tests: live LLM with recording, or replay from saved traces.
//!
//! These tests exercise the full agent loop with real tool execution.
//!
//! # Running
//!
//! **Replay mode** (deterministic, needs committed trace fixture):
//! ```bash
//! cargo test --features libsql --test e2e_live -- --ignored
//! ```
//!
//! **Live mode** (real LLM calls, records/updates trace fixture):
//! ```bash
//! IRONCLAW_LIVE_TEST=1 cargo test --features libsql --test e2e_live -- --ignored
//! ```
//!
//! See `tests/support/live_harness.rs` for the harness documentation.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod live_tests {
    use std::time::Duration;

    use ironclaw::channels::StatusUpdate;

    use crate::support::live_harness::{LiveTestHarness, LiveTestHarnessBuilder};

    const ZIZMOR_JUDGE_CRITERIA: &str = "\
        The response contains a zizmor security scan report for GitHub Actions \
        workflows. It lists findings with severity levels (error, warning, etc.). \
        It mentions specific finding types such as template-injection, artipacked, \
        excessive-permissions, dangerous-triggers, or similar GitHub Actions \
        security issues.";

    /// Shared logic for zizmor scan tests (v1 and v2 engines).
    async fn run_zizmor_scan(harness: LiveTestHarness) {
        let user_input = "can we run https://github.com/zizmorcore/zizmor";
        let rig = harness.rig();
        rig.send_message(user_input).await;

        let responses = rig.wait_for_responses(1, Duration::from_secs(300)).await;

        assert!(!responses.is_empty(), "Expected at least one response");

        let text: Vec<String> = responses.iter().map(|r| r.content.clone()).collect();
        let tools = rig.tool_calls_started();

        // Log diagnostics before asserting.
        eprintln!("[ZizmorScan] Tools used: {tools:?}");
        eprintln!(
            "[ZizmorScan] Response preview: {}",
            text.join("\n").chars().take(500).collect::<String>()
        );

        // The agent should have used the shell tool to install/run zizmor.
        assert!(
            tools.iter().any(|t| t == "shell"),
            "Expected shell tool to be used for running zizmor, got: {tools:?}"
        );

        let joined = text.join("\n").to_lowercase();

        // The response should mention zizmor and contain scan findings.
        assert!(
            joined.contains("zizmor"),
            "Response should mention zizmor: {joined}"
        );

        // LLM judge for semantic verification (live mode only).
        if let Some(verdict) = harness.judge(&text, ZIZMOR_JUDGE_CRITERIA).await {
            assert!(verdict.pass, "LLM judge failed: {}", verdict.reasoning);
        }

        harness.finish(user_input, &text).await;
    }

    /// Zizmor scan via engine v1 (default agentic loop).
    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys or a recorded trace fixture
    async fn zizmor_scan() {
        let harness = LiveTestHarnessBuilder::new("zizmor_scan")
            .with_max_tool_iterations(40)
            .with_auto_approve_tools(true)
            .build()
            .await;

        run_zizmor_scan(harness).await;
    }

    /// Zizmor scan via engine v2.
    ///
    /// NOTE: Engine v2 does not yet honor `auto_approve_tools` from config —
    /// it only checks the per-session "always" set. This means tool calls
    /// that require approval (shell, file_write, etc.) will be paused.
    /// The test currently validates that v2 at least attempts the task and
    /// mentions zizmor in its response (even if it can't execute shell).
    /// When v2 gains auto-approve support, update this to use `run_zizmor_scan`.
    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys or a recorded trace fixture
    async fn zizmor_scan_v2() {
        let harness = LiveTestHarnessBuilder::new("zizmor_scan_v2")
            .with_engine_v2(true)
            .with_max_tool_iterations(40)
            .build()
            .await;

        let user_input = "can we run https://github.com/zizmorcore/zizmor";
        let rig = harness.rig();
        rig.send_message(user_input).await;

        let responses = rig.wait_for_responses(1, Duration::from_secs(300)).await;

        assert!(!responses.is_empty(), "Expected at least one response");

        let text: Vec<String> = responses.iter().map(|r| r.content.clone()).collect();
        let tools = rig.tool_calls_started();

        eprintln!("[ZizmorScanV2] Tools used: {tools:?}");
        eprintln!(
            "[ZizmorScanV2] Response preview: {}",
            text.join("\n").chars().take(500).collect::<String>()
        );

        let joined = text.join("\n").to_lowercase();

        // V2 without auto-approve hits an approval gate for shell/tool_install.
        // The response may be the approval prompt itself rather than agent output.
        // Verify the agent at least attempted a relevant action.
        let attempted_relevant_tool = tools.iter().any(|t| {
            t == "shell"
                || t == "tool_install"
                || t.starts_with("tool_search")
                || t.starts_with("skill_search")
        });
        assert!(
            attempted_relevant_tool,
            "Expected agent to attempt a relevant tool, got: {tools:?}"
        );

        // The response should mention zizmor or approval (approval gate).
        assert!(
            joined.contains("zizmor") || joined.contains("approval"),
            "Response should mention zizmor or approval: {joined}"
        );

        harness.finish(user_input, &text).await;
    }

    /// Diagnostic live test: drive the agent through a Google Drive lookup
    /// against the user's actual `~/.ironclaw` setup (real WASM tools registered
    /// from `~/.ironclaw/tools/`, real LLM creds from `~/.ironclaw/.env`).
    ///
    /// This test exists to *surface* the rough edges around the
    /// `google-drive-tool` extension, not to assert a happy-path success. The
    /// user reported three observable failure modes from an interactive REPL
    /// session:
    ///
    /// 1. The auth-readiness probe canonicalises `google-drive-tool` to
    ///    `google_drive_tool` and then complains that the WASM file isn't on
    ///    disk under that name (the file is `google-drive-tool.wasm`):
    ///    `Extension not installed: WASM tool 'google_drive_tool' not found`.
    ///    The probe then logs `treating tool as ready` and the call proceeds —
    ///    so the warning is misleading rather than fatal, but it's also a sign
    ///    that the legacy hyphen alias isn't being consulted in this code path
    ///    (`src/bridge/auth_manager.rs`).
    ///
    /// 2. The agent sometimes invokes the tool without `file_id` (e.g. trying
    ///    to call a list/search action through the same dispatch surface) and
    ///    gets back `Sandbox error: Tool error: Invalid parameters: missing
    ///    field 'file_id'` after ~4ms. The agent then has to retry with the
    ///    proper schema. This is the user-visible manifestation of the
    ///    "transient first-call failure, second-call succeeds" pattern.
    ///
    /// 3. With a fresh test database (no OAuth secret for `google_oauth_token`),
    ///    the tool either errors during HTTP credential injection or returns a
    ///    structured error payload. We don't make hard claims about *which*
    ///    here — the test dumps everything for inspection.
    ///
    /// The test does NOT assert that the agent successfully retrieves the doc:
    /// the test rig uses a temp libSQL DB so the user's real Drive OAuth token
    /// isn't accessible. The point is to exercise the same code path the user
    /// hit interactively and capture the diagnostic surface.
    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys + real ~/.ironclaw/tools/ setup
    async fn george_one_on_one_drive_lookup() {
        let harness = LiveTestHarnessBuilder::new("george_one_on_one_drive_lookup")
            .with_max_tool_iterations(40)
            .with_auto_approve_tools(true)
            .build()
            .await;

        // Direct the agent at Google Drive explicitly. The original
        // user-facing prompt was vaguer ("read through latest George 1:1
        // meeting notes...") and the fresh test workspace lets the agent give
        // up after a memory_search miss without ever touching Drive — which
        // means the Drive bug surface never gets exercised. By naming the
        // tool directly we force the agent through `google-drive-tool`'s
        // action dispatch and credential injection paths, which is where the
        // user reported the failure.
        let user_input = "Use the google-drive-tool to find a Google Doc \
                          titled '1:1 George <> Illia' in my Drive, read the \
                          latest meeting notes from it, then give me \
                          executive-coach-level feedback on how I could \
                          conduct these 1:1s better.";
        let rig = harness.rig();
        rig.send_message(user_input).await;

        let responses = rig.wait_for_responses(1, Duration::from_secs(300)).await;

        let text: Vec<String> = responses.iter().map(|r| r.content.clone()).collect();
        let tools_started = rig.tool_calls_started();
        let tools_completed = rig.tool_calls_completed();
        let status_events = rig.captured_status_events();

        // ── Diagnostics ─────────────────────────────────────────────────────
        eprintln!("[George1on1] Tools started ({}):", tools_started.len());
        for name in &tools_started {
            eprintln!("  ● {name}");
        }

        eprintln!("[George1on1] Tools completed ({}):", tools_completed.len());
        for (name, success) in &tools_completed {
            let mark = if *success { "✓" } else { "✗" };
            eprintln!("  {mark} {name}");
        }

        // Surface every tool failure with parameters + error so we can see
        // exactly what went wrong on each call. This is the most valuable
        // signal for debugging the "missing field `file_id`" path.
        eprintln!("[George1on1] Tool failures (with params + error):");
        for ev in &status_events {
            if let StatusUpdate::ToolCompleted {
                name,
                success: false,
                error,
                parameters,
            } = ev
            {
                eprintln!("  ─── {name} ───");
                if let Some(err) = error {
                    eprintln!("    error: {err}");
                }
                if let Some(params) = parameters {
                    eprintln!("    params: {params}");
                }
            }
        }

        eprintln!("[George1on1] Response preview ({} chunks):", text.len());
        for (i, chunk) in text.iter().enumerate() {
            let preview: String = chunk.chars().take(800).collect();
            eprintln!("  [{i}] {preview}");
        }

        // ── Soft assertions ─────────────────────────────────────────────────
        // We don't assert success — the test rig has no OAuth secrets in its
        // temp DB, so a happy-path lookup is impossible. We assert only that
        // the agent *engaged* with the task in a way that exercises the
        // suspected bug surface.

        assert!(
            !responses.is_empty(),
            "Expected at least one agent response (even if it's a 'cannot find' message)"
        );

        // The agent should have at least attempted a tool that could plausibly
        // locate the meeting notes: Drive, Docs, Gmail, Calendar, memory
        // search, web search, or file system. If none of these fired, the
        // agent gave up too early and the trace isn't useful.
        let attempted_lookup = tools_started.iter().any(|t| {
            t.contains("drive")
                || t.contains("docs")
                || t.contains("gmail")
                || t.contains("calendar")
                || t.starts_with("memory")
                || t.starts_with("web_search")
                || t.starts_with("file_")
        });
        assert!(
            attempted_lookup,
            "Expected the agent to attempt at least one lookup tool, got: {tools_started:?}"
        );

        // If the agent called google-drive-tool and it failed with the
        // "missing field `file_id`" pattern, surface that explicitly so the
        // failure jumps out in CI logs even when the test passes.
        let drive_param_bug = status_events.iter().any(|ev| {
            matches!(ev, StatusUpdate::ToolCompleted {
                name,
                success: false,
                error: Some(err),
                ..
            } if name.contains("drive") && err.contains("file_id"))
        });
        if drive_param_bug {
            eprintln!(
                "[George1on1] ⚠ REPRODUCED: google-drive-tool failed with \
                 'missing field `file_id`' — see params dump above. \
                 The agent's first call to the tool used an action that does \
                 not require file_id (e.g. list/search), but the WASM tool's \
                 schema rejected the call. Investigate the action dispatch in \
                 tools-src/google-drive-tool/."
            );
        }

        harness.finish(user_input, &text).await;
    }
}
