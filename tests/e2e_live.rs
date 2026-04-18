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

    /// End-to-end round-trip test for the post-flight auth gate.
    ///
    /// Phase A: starts the rig with NO Google credentials in the temp
    /// DB (the harness uses `with_secrets([])` semantics — by default
    /// nothing is seeded), sends an NVIDIA GTC Drive search prompt,
    /// and asserts that the agent emits a `StatusUpdate::AuthRequired`
    /// within one iteration. The expected path is:
    ///
    ///   1. Agent calls `google-drive-tool { action: "list_files" }`
    ///   2. WASM wrapper's `resolve_host_credentials` reports
    ///      `missing_required = ["google_oauth_token"]`
    ///   3. Wrapper fails closed with
    ///      `"WASM tool '...' requires credentials that are not configured"`
    ///   4. `effect_adapter::execute_action_internal`'s post-flight
    ///      branch runs `auth::postflight::detect_post_call_auth_failure`
    ///   5. The matcher fires (commit cd8b68de added the
    ///      `requires credentials + not configured` pair)
    ///   6. The detector calls `ensure_extension_ready(.., ExplicitAuth)`
    ///      → `EnsureReadyOutcome::NeedsAuth`
    ///   7. `EngineError::GatePaused { resume_kind: Authentication }`
    ///      bubbles to the orchestrator
    ///   8. Router stores it in `pending_gates` and emits
    ///      `StatusUpdate::AuthRequired` to the channel
    ///
    /// Phase B: directly inserts a synthetic credential via
    /// `secrets_store()`, sends the synthetic value as a follow-up
    /// message. The v2 router treats the next user message after an
    /// auth gate as `GateResolution::CredentialProvided`, which calls
    /// `submit_auth_token` (idempotent overwrite of what we just
    /// inserted), then `execute_pending_gate_action` which in turn
    /// calls `execute_resolved_pending_action`. The original Drive
    /// call replays. We don't assert success against the real Google
    /// API — the synthetic token will be rejected — we just assert
    /// the resume path *ran* (visible via additional tool activity
    /// and a follow-up response).
    ///
    /// Uses NVIDIA GTC keynote as the search target so any captured
    /// trace fixtures contain only public conference content.
    #[tokio::test]
    #[ignore] // Live tier: requires real Google OAuth credentials in the
    // developer's `~/.ironclaw/ironclaw.db`. Live-only on purpose: the
    // recorded trace would inevitably capture the bearer token, real
    // Drive file metadata, and HTTP headers — all of which are PII
    // that's hard to scrub safely. The test runs against the developer's
    // real environment in live mode and is skipped otherwise. Hermetic
    // regression coverage for the underlying alias-aware capabilities
    // bug lives in `test_auth_wasm_tool_finds_legacy_hyphen_alias`.
    async fn drive_auth_gate_roundtrip() {
        use crate::support::live_harness::TestMode;
        use ironclaw::channels::StatusUpdate;

        let harness = LiveTestHarnessBuilder::new("drive_auth_gate_roundtrip")
            .with_engine_v2(true)
            .with_max_tool_iterations(20)
            .with_auto_approve_tools(true)
            // Seed the real Google OAuth credentials from the
            // developer's libSQL DB so Phase B's resume can actually
            // call the real Google Drive API and produce real data —
            // not just a synthetic placeholder.
            //
            // Three companion records are needed:
            //
            //   * `google_oauth_token` — the access token itself.
            //     Deleted before Phase A so the gate fires; the value
            //     is captured first and re-sent as the gate resolution
            //     in Phase B.
            //   * `google_oauth_token_refresh_token` — the refresh
            //     token. Also deleted before Phase A so Phase A
            //     doesn't transparently auto-refresh and skip the
            //     gate (`maybe_refresh_before_read` would otherwise
            //     re-create the access token from this).
            //   * `google_oauth_token_scopes` — the recorded set of
            //     OAuth scopes the access token grants. Kept across
            //     the test. Without it, `auth_wasm_tool`'s
            //     `needs_scope_expansion` check fires for the
            //     re-stored access token in Phase B and forces a
            //     full re-auth, which would mask the auto-retry path
            //     we're trying to verify.
            .with_secrets([
                "google_oauth_token",
                "google_oauth_token_refresh_token",
                "google_oauth_token_scopes",
            ])
            // No committed trace fixture — see test attribute comment.
            .with_no_trace_recording()
            .build()
            .await;

        // Live-mode only. In replay mode the harness builds a stub rig
        // (no recorded fixture, no LLM provider) and we exit early.
        if harness.mode() != TestMode::Live {
            eprintln!(
                "[DriveAuthGate] Live-only test — skipping outside `IRONCLAW_LIVE_TEST=1`. \
                 Hermetic regression covered by \
                 `test_auth_wasm_tool_finds_legacy_hyphen_alias`."
            );
            return;
        }

        let rig = harness.rig();

        let secrets = rig
            .secrets_store()
            .expect(
                "drive_auth_gate_roundtrip requires a secrets store; \
                 ensure ~/.ironclaw/.env has SECRETS_MASTER_KEY or the OS keychain entry",
            )
            .clone();
        let owner = rig.owner_id().to_string();

        // Capture the real OAuth token value so Phase B can re-send it
        // as the gate resolution. The router treats the next user
        // message after an auth gate as the credential value, so we
        // need to send the real token (or a synthetic one) to drive
        // the resume path. The plaintext never leaves the test process.
        //
        // If the developer's stored token is expired, fall back to a
        // synthetic token. The auto-retry verification (LLM call count
        // <= 2) still works in that case — the only thing we lose is
        // the actual Drive API call succeeding. The test prints which
        // mode it's running in so the developer can refresh their
        // token via normal `ironclaw` usage if they want full coverage.
        let real_token_result = secrets.get_decrypted(&owner, "google_oauth_token").await;
        let (resume_token, real_token_used) = match real_token_result {
            Ok(decrypted) => {
                eprintln!(
                    "[DriveAuthGate] Captured real google_oauth_token ({} chars) for Phase B",
                    decrypted.expose().len()
                );
                (decrypted.expose().to_string(), true)
            }
            Err(ironclaw::secrets::SecretError::Expired) => {
                eprintln!(
                    "[DriveAuthGate] WARNING: stored google_oauth_token is EXPIRED. \
                     Falling back to a synthetic token for Phase B — the resume path \
                     will run end-to-end but the Drive API call will fail with 401, \
                     not return real data. To get full real-data coverage, run \
                     `ironclaw` once normally (any prompt that touches Drive) so the \
                     OAuth refresh flow refreshes the token in your real DB, then \
                     re-run this test."
                );
                (
                    "drive-auth-gate-roundtrip-synthetic-token".to_string(),
                    false,
                )
            }
            Err(e) => panic!(
                "Unexpected error reading seeded google_oauth_token: {e}. \
                 Ensure your developer DB at ~/.ironclaw/ironclaw.db has the credential."
            ),
        };

        // Defensive: delete BOTH the access token and the refresh
        // token sibling. The OAuth refresh path
        // (`auth::mod::maybe_refresh_before_read`) checks for a sibling
        // `<name>_refresh_token` and will trigger a refresh that
        // re-creates the access token if either is present.
        let _ = secrets.delete(&owner, "google_oauth_token").await;
        let _ = secrets
            .delete(&owner, "google_oauth_token_refresh_token")
            .await;
        eprintln!("[DriveAuthGate] Phase A: temp DB has no google_oauth_token");

        // Snapshot LLM call count BEFORE Phase A so we can prove the
        // post-AuthCompleted retry happens with NO additional LLM call
        // between AuthCompleted and the tool retry.
        let baseline_llm_calls = rig.llm_call_count();

        // ── Phase A: send the prompt and wait for the auth gate ───────
        let user_input = "Find the NVIDIA GTC keynote presentation in my Google Drive \
                          and summarize the key announcements";
        rig.send_message(user_input).await;

        let phase_a_responses = rig.wait_for_responses(1, Duration::from_secs(120)).await;
        let phase_a_text: Vec<String> = phase_a_responses
            .iter()
            .map(|r| r.content.clone())
            .collect();
        let phase_a_tools = rig.tool_calls_started();
        let phase_a_status = rig.captured_status_events();
        let phase_a_llm_calls = rig.llm_call_count() - baseline_llm_calls;

        eprintln!(
            "[DriveAuthGate][Phase A] LLM calls: {phase_a_llm_calls}, Tools attempted ({}): {phase_a_tools:?}",
            phase_a_tools.len()
        );
        eprintln!(
            "[DriveAuthGate][Phase A] Response preview: {}",
            phase_a_text
                .join("\n")
                .chars()
                .take(500)
                .collect::<String>()
        );

        // Phase A should be a single LLM call: the LLM generates a
        // tool_call, the pre-flight gate intercepts it, no further
        // LLM calls happen until the user provides credentials.
        assert_eq!(
            phase_a_llm_calls, 1,
            "Phase A: expected exactly 1 LLM call (the tool-call generation), \
             got {phase_a_llm_calls}. More than 1 means the agent went into a \
             recovery loop instead of pausing immediately on the auth gate."
        );

        // Collect every AuthRequired event so we can assert *which*
        // extension fired the gate. Before the post-flight detector
        // landed, the agent would silently fall back to a
        // tool_install/web_search recovery loop and trigger an
        // AuthRequired for `brave_api_key` instead of pausing on the
        // Drive failure. A loose `is_some()` check would let that
        // regression slip through.
        let auth_required_events: Vec<_> = phase_a_status
            .iter()
            .filter_map(|s| match s {
                StatusUpdate::AuthRequired {
                    extension_name,
                    instructions,
                    auth_url,
                    ..
                } => Some((
                    extension_name.clone(),
                    instructions.clone(),
                    auth_url.clone(),
                )),
                _ => None,
            })
            .collect();
        let drive_gate = auth_required_events
            .iter()
            .find(|(ext, _, _)| ext.as_str().contains("google") || ext.as_str().contains("drive"));
        assert!(
            drive_gate.is_some(),
            "Phase A: expected an AuthRequired event for the Google Drive extension, \
             but got: {auth_required_events:?}. The post-flight detector should have \
             paused on the Drive failure directly instead of letting the agent run a \
             recovery loop into a different extension."
        );
        let (gate_extension, _gate_instructions, gate_auth_url) = drive_gate.unwrap().clone();
        eprintln!(
            "[DriveAuthGate][Phase A] AuthRequired fired: extension={gate_extension}, \
             auth_url present={}",
            gate_auth_url.is_some()
        );

        // The agent must NOT have run a tool_install / tool_activate
        // recovery loop — that's the bad behaviour the post-flight
        // detector eliminates.
        let bad_recovery = phase_a_tools
            .iter()
            .any(|t| t == "tool_install" || t == "tool_activate" || t == "tool-install");
        assert!(
            !bad_recovery,
            "Phase A: agent ran a tool_install/tool_activate recovery loop instead \
             of pausing for auth on the first iteration. Tools attempted: {phase_a_tools:?}"
        );

        // ── Phase B: send the real token as the gate resolution ──────
        // The router treats the next user message after an auth gate
        // as `GateResolution::CredentialProvided { token }` and feeds
        // it through `submit_auth_token`. With the real Drive OAuth
        // token, the production resume path then runs:
        //   1. submit_auth_token writes the credential under the
        //      WASM tool's declared `auth.secret_name`.
        //   2. AuthCompleted status is emitted to the channel.
        //   3. execute_pending_gate_action loads the original tool call.
        //   4. execute_resolved_pending_action re-runs the tool through
        //      execute_action_internal — directly, with no LLM hop.
        //   5. The tool executes against the real Google Drive API and
        //      returns real data.
        //   6. resume_thread feeds the result back to the engine, which
        //      makes a SECOND LLM call to summarise the result for the
        //      user.
        //
        // The expected total LLM call count is therefore exactly 2:
        // one for the initial tool_call, one for the summary. Anything
        // higher would indicate that the LLM was consulted between
        // AuthCompleted and the tool retry, which is exactly the
        // behaviour the post-flight gate work was meant to eliminate.
        eprintln!(
            "[DriveAuthGate] Phase B: sending {} as gate resolution",
            if real_token_used {
                "real google_oauth_token"
            } else {
                "synthetic placeholder token"
            }
        );
        rig.send_message(&resume_token).await;

        let total_responses = rig.wait_for_responses(2, Duration::from_secs(180)).await;
        assert!(
            total_responses.len() >= 2,
            "Phase B: expected a follow-up response after credential resolution; \
             got {} response(s) total",
            total_responses.len()
        );

        let phase_b_text: Vec<String> = total_responses
            .iter()
            .skip(phase_a_responses.len())
            .map(|r| r.content.clone())
            .collect();
        let total_llm_calls = rig.llm_call_count() - baseline_llm_calls;
        let phase_b_llm_calls = total_llm_calls - phase_a_llm_calls;

        eprintln!(
            "[DriveAuthGate][Phase B] LLM calls: {phase_b_llm_calls}, Total: {total_llm_calls}"
        );
        eprintln!(
            "[DriveAuthGate][Phase B] Response preview: {}",
            phase_b_text
                .join("\n")
                .chars()
                .take(500)
                .collect::<String>()
        );

        assert!(
            !phase_b_text.is_empty(),
            "Phase B: response must not be empty after credential resolution"
        );

        // The CRITICAL assertion: the auto-retry path must NOT go back
        // through the LLM to recover from a missing credential. Tool
        // calls between `AuthCompleted` and the resumed tool execution
        // are kernel-driven, not LLM-driven.
        //
        // Distinguishing the auto-retry from a recovery loop is the
        // job of the *first tool call after Phase A*. The pre-fix
        // recovery loop signature was the LLM choosing to call
        // `secret_list`, `tool_search`, `tool_install`, etc. — never
        // re-attempting the original `google_drive_tool` call. The
        // auto-retry signature is the original `google_drive_tool`
        // call running again, kernel-side, before any new LLM
        // iteration touches the recovery tools.
        //
        // We assert two things:
        //
        //   1. No `tool_install` / `tool_activate` / `secret_list`
        //      tool ever appears in Phase B's tool activity (the
        //      pre-fix recovery loop's smoking gun).
        //
        //   2. The first tool that does run in Phase B is one of
        //      `google_drive_tool` or its action variants (so we know
        //      the resume actually re-ran the gated action and the
        //      LLM didn't get a second chance to pick something else).
        let phase_b_tools = rig
            .tool_calls_started()
            .into_iter()
            .skip(phase_a_tools.len())
            .collect::<Vec<_>>();
        eprintln!(
            "[DriveAuthGate][Phase B] Tools attempted ({}): {phase_b_tools:?}",
            phase_b_tools.len()
        );
        let phase_b_recovery = phase_b_tools.iter().any(|t| {
            t == "tool_install"
                || t == "tool-install"
                || t == "tool_activate"
                || t == "secret_list"
                || t.starts_with("tool_search")
        });
        assert!(
            !phase_b_recovery,
            "Phase B contains pre-fix recovery-loop tools — the resume should \
             re-run google_drive_tool through the kernel, not delegate to the \
             LLM to figure out what to do. Phase B tools: {phase_b_tools:?}"
        );
        if real_token_used {
            // Real token: the resume must have actually re-run
            // google_drive_tool *first* (before any other tool), and
            // it must have executed *successfully* against the real
            // Google Drive API. The agent's eventual response text
            // can vary widely (full summary, partial summary, hits a
            // different extension's auth gate, etc.), so we don't
            // pattern-match on it — we look at the captured tool
            // events instead, which are deterministic about which
            // tools ran and whether they succeeded.
            let first_phase_b_tool = phase_b_tools.first().map(String::as_str);
            assert!(
                first_phase_b_tool.is_some_and(|t| t.contains("google_drive")),
                "Phase B's first tool call must be google_drive_tool (the auto-retry \
                 of the originally-gated action), got {first_phase_b_tool:?}. If this \
                 is anything else, the resume didn't re-run the original action — \
                 the LLM picked a different tool, which is the pre-fix recovery loop \
                 signature."
            );

            // At least one google_drive_tool execution must have *succeeded*
            // (not just been attempted) — that's our proof that the auto-retry
            // actually called the real Google Drive API and got data back.
            // `tool_calls_completed` returns (name, success) for every
            // ToolCompleted status event.
            let drive_succeeded = rig
                .tool_calls_completed()
                .into_iter()
                .skip(phase_a_tools.len())
                .any(|(name, success)| name.contains("google_drive") && success);
            assert!(
                drive_succeeded,
                "Real token in use but no google_drive_tool execution succeeded in \
                 Phase B. The auto-retry either re-fired the gate or the wrapper \
                 rejected the credential. Phase B tools: {phase_b_tools:?}"
            );

            assert!(
                phase_b_llm_calls >= 1,
                "Real token in use but Phase B made 0 LLM calls — the auto-retry \
                 either failed or paused at another gate before the LLM was \
                 consulted to summarise the result. Phase B response: {phase_b_text:?}"
            );
        }

        // Hand both turns to the harness so the .log file shows the
        // full session for documentation. The user message in Phase B
        // is replaced with a placeholder so the real token never
        // appears on disk (even though we don't commit a trace, the
        // .log file may still be diffed by developers).
        let phase_b_user_label = if real_token_used {
            "<real google oauth token redacted>".to_string()
        } else {
            "<synthetic auth token>".to_string()
        };
        let turns = vec![
            (user_input.to_string(), phase_a_text.clone()),
            (phase_b_user_label, phase_b_text.clone()),
        ];
        harness.finish_turns_simple(&turns).await;
    }

    /// End-to-end verification of the *transparent* OAuth refresh path.
    ///
    /// Where `drive_auth_gate_roundtrip` exercises the gate-fire +
    /// resume sequence (the agent has no credential and the user has
    /// to provide one), this scenario exercises the path where the
    /// agent has an *expired* access token plus a valid refresh token
    /// — the wrapper's credential resolution
    /// (`auth::mod::maybe_refresh_before_read`) should detect the
    /// expiration, refresh the token via the OAuth provider's token
    /// endpoint, and proceed transparently. **No auth gate should ever
    /// fire and the user should never see an "Authentication required"
    /// prompt.**
    ///
    /// This is the most common production case for an active user: the
    /// access token expired since the last interaction, the refresh
    /// token is still valid, the next Drive call should just work.
    ///
    /// Live-only (no committed trace fixture) for the same reason as
    /// `drive_auth_gate_roundtrip`: the recorded HTTP exchanges and
    /// LLM input would inevitably contain the bearer token, real
    /// Drive file metadata, and PII. Hermetic regression coverage for
    /// the underlying refresh mechanism lives in
    /// `auth::tests::*` (the `maybe_refresh_before_read` unit tests).
    #[tokio::test]
    #[ignore] // Live tier: requires real Google OAuth credentials in the
    // developer's `~/.ironclaw/ironclaw.db` (must include the refresh
    // token sibling so the wrapper can refresh server-side).
    async fn drive_transparent_oauth_refresh() {
        use crate::support::live_harness::TestMode;
        use ironclaw::channels::StatusUpdate;

        let harness = LiveTestHarnessBuilder::new("drive_transparent_oauth_refresh")
            .with_engine_v2(true)
            .with_max_tool_iterations(20)
            .with_auto_approve_tools(true)
            .with_secrets([
                "google_oauth_token",
                "google_oauth_token_refresh_token",
                "google_oauth_token_scopes",
            ])
            .with_no_trace_recording()
            .build()
            .await;

        if harness.mode() != TestMode::Live {
            eprintln!(
                "[DriveRefresh] Live-only test — skipping outside `IRONCLAW_LIVE_TEST=1`. \
                 Hermetic regression for the OAuth refresh layer lives in \
                 `auth::tests::*` and `test_auth_wasm_tool_finds_legacy_hyphen_alias`."
            );
            return;
        }

        let rig = harness.rig();
        let secrets = rig
            .secrets_store()
            .expect("drive_transparent_oauth_refresh requires a secrets store")
            .clone();
        let owner = rig.owner_id().to_string();

        // Verify the seeded data shape: we need BOTH the access token
        // (whatever its expiration) AND the refresh token to be
        // present. The whole point of this test is "wrapper auto-refreshes
        // when access token is expired", so if either is missing the
        // test setup is broken — fail with a clear message instead of
        // silently passing or hitting a misleading downstream error.
        let access_present = secrets
            .exists(&owner, "google_oauth_token")
            .await
            .unwrap_or(false);
        let refresh_present = secrets
            .exists(&owner, "google_oauth_token_refresh_token")
            .await
            .unwrap_or(false);
        assert!(
            access_present && refresh_present,
            "drive_transparent_oauth_refresh requires both \
             `google_oauth_token` and `google_oauth_token_refresh_token` to be \
             seeded from the developer DB. access={access_present}, \
             refresh={refresh_present}. Run `ironclaw` once with a Drive prompt \
             so the OAuth flow stores both records, then re-run this test."
        );

        eprintln!("[DriveRefresh] Seeded credentials: access + refresh + scopes");

        // Send the prompt. The wrapper should:
        //   1. Pre-flight `auth_wasm_tool` sees `secrets.exists()` =
        //      true, scope check passes, returns Authenticated.
        //   2. Tool runs → `resolve_host_credentials` →
        //      `resolve_secret_for_runtime` → `maybe_refresh_before_read`.
        //   3. `store.get()` returns `Expired` (if the access token is
        //      expired) → triggers refresh via the refresh sibling.
        //   4. Refresh succeeds → fresh token written → wrapper uses it.
        //   5. Drive API call succeeds.
        //   6. Engine summarises.
        //
        // (If the access token happens to still be valid at test time,
        // step 3 just uses it directly. The test still passes — we're
        // verifying "no gate fires", which holds in both cases.)
        let user_input = "Find the NVIDIA GTC keynote presentation in my Google Drive \
                          and summarize the key announcements";
        rig.send_message(user_input).await;

        let responses = rig.wait_for_responses(1, Duration::from_secs(180)).await;
        let response_text: Vec<String> = responses.iter().map(|r| r.content.clone()).collect();
        let tools = rig.tool_calls_started();
        let status = rig.captured_status_events();

        eprintln!(
            "[DriveRefresh] Tools attempted ({}): {tools:?}",
            tools.len()
        );
        eprintln!(
            "[DriveRefresh] Response preview: {}",
            response_text
                .join("\n")
                .chars()
                .take(500)
                .collect::<String>()
        );

        // CRITICAL assertion #1: NO AuthRequired event for the Google
        // extension. The whole point of this test is that the refresh
        // is transparent — the user should never see an auth prompt.
        let google_auth_gate = status.iter().find(|s| {
            matches!(
                s,
                StatusUpdate::AuthRequired { extension_name, .. }
                    if extension_name.as_str().contains("google")
                        || extension_name.as_str().contains("drive")
            )
        });
        assert!(
            google_auth_gate.is_none(),
            "Transparent refresh failed: an AuthRequired event fired for the \
             Google extension. The wrapper's `maybe_refresh_before_read` should \
             have refreshed the token via the refresh_token sibling without \
             ever surfacing a gate. Status events: {:?}",
            status
                .iter()
                .filter_map(|s| match s {
                    StatusUpdate::AuthRequired { extension_name, .. } => {
                        Some(format!("AuthRequired({extension_name})"))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        );

        // CRITICAL assertion #2: at least one google_drive_tool call
        // succeeded against the real API. If the refresh failed
        // silently (e.g. invalid refresh token, hosted-proxy
        // misconfig), the wrapper would have failed closed and the
        // post-flight detector would have fired the gate — caught by
        // assertion #1. If somehow neither happened but the tool
        // never ran either, this catches that.
        let drive_succeeded = rig
            .tool_calls_completed()
            .into_iter()
            .any(|(name, success)| name.contains("google_drive") && success);
        assert!(
            drive_succeeded,
            "No google_drive_tool execution succeeded — the wrapper either \
             never ran the tool or it failed silently. Tools attempted: {tools:?}"
        );

        // CRITICAL assertion #3: the response must not be a Drive
        // auth-required prompt. We allow other extensions' gates to
        // fire (e.g. the agent may try web_search after pulling Drive
        // content and hit brave_api_key) — that's a *different*
        // extension and unrelated to the Google refresh path we're
        // verifying.
        let joined = response_text.join("\n").to_lowercase();
        let drive_gate_phrases = [
            "authentication required for 'google",
            "authentication required for \"google",
            "authentication required for google",
        ];
        let drive_auth_in_response = drive_gate_phrases.iter().any(|p| joined.contains(p));
        assert!(
            !drive_auth_in_response,
            "Response contains a Google-extension auth prompt — the transparent \
             refresh path failed and surfaced a gate to the user. \
             Response: {response_text:?}"
        );

        let turns = vec![(user_input.to_string(), response_text.clone())];
        harness.finish_turns_simple(&turns).await;
    }
}
