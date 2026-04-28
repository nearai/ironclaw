//! Live end-to-end test for the engine v2 per-project sandbox.
//!
//! This test proves that the entire sandbox path — mount table routing,
//! `ContainerizedFilesystemBackend`, `ProjectSandboxManager`, Docker exec
//! session, and the in-container `sandbox_daemon` — works under a real
//! agent driving a real LLM. It is the manual-verification replacement:
//! "clone ironclaw, rename to megaclaw, run cargo check" as a single
//! asserted scenario.
//!
//! # Running
//!
//! **Live mode (real LLM + real Docker):**
//! ```bash
//! # 1) build the sandbox image once
//! docker build -f crates/Dockerfile.sandbox -t ironclaw/sandbox:dev .
//!
//! # 2) run the test
//! SANDBOX_ENABLED=true IRONCLAW_LIVE_TEST=1 \
//!   cargo test --features libsql --test sandbox_live_e2e -- --ignored --nocapture
//! ```
//!
//! **Replay mode:**
//! ```bash
//! SANDBOX_ENABLED=true \
//!   cargo test --features libsql --test sandbox_live_e2e -- --ignored --nocapture
//! ```
//!
//! Both modes require Docker + the sandbox image — the actual filesystem
//! side effects happen inside a real container either way. The difference
//! is whether the LLM calls are recorded (live) or replayed from a
//! committed trace fixture (replay). If Docker or the image is unavailable
//! the test skips with a helpful message rather than failing.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod sandbox_e2e_tests {
    use std::time::Duration;

    use crate::support::live_harness::LiveTestHarnessBuilder;

    /// Prints a skip reason and returns from the test.
    macro_rules! skip {
        ($($arg:tt)*) => {
            eprintln!("[SandboxE2E] SKIP: {}", format!($($arg)*));
            return;
        };
    }

    async fn docker_reachable() -> bool {
        ironclaw::sandbox::connect_docker().await.is_ok()
    }

    async fn sandbox_image_present(image: &str) -> bool {
        match ironclaw::sandbox::connect_docker().await {
            Ok(docker) => docker.inspect_image(image).await.is_ok(),
            Err(_) => false,
        }
    }

    /// The judge criteria: the agent must report that it cloned the repo,
    /// performed the rename, and verified it with grep. Used in live mode
    /// only — replay mode has no judge provider.
    const JUDGE_CRITERIA: &str = "\
        The assistant reports that it (1) successfully cloned a repository \
        into /project/repo, (2) renamed occurrences of 'ironclaw' to \
        'megaclaw' in at least one file (typically Cargo.toml), and (3) \
        verified the rename by grepping for 'megaclaw' and finding it. \
        All three steps must be mentioned with concrete evidence \
        (command output, line number, or file path).";

    /// Live/replay end-to-end: the agent is asked to clone ironclaw into the
    /// sandbox, rename it to megaclaw, and run a cargo check. We assert that:
    ///
    /// 1. The `shell` tool was actually used (tool_calls_started recorded it),
    /// 2. The agent's final response mentions `megaclaw`,
    /// 3. (live only) The LLM judge signs off on the scenario.
    ///
    /// What we *don't* assert (and why): the exact cargo check outcome.
    /// Cloning the full ironclaw workspace and building it from scratch
    /// inside a cold container is minutes of work with many non-deterministic
    /// network steps (crates.io, git submodules, rustup). The goal of this
    /// test is to prove the sandbox plumbing works end-to-end; cargo check's
    /// success/failure is incidental, and the agent's summary is what we
    /// verify. Phase 7 polish can add a "persistence across stop/start"
    /// assertion once the idle reaper lands.
    #[tokio::test]
    #[ignore] // Live tier: needs Docker + sandbox image + (in live mode) LLM keys
    async fn sandbox_clones_ironclaw_and_renames_to_megaclaw() {
        // Skip cleanly when the test environment can't run this scenario.
        // These are not failures — the test is opt-in and requires setup.
        if !docker_reachable().await {
            skip!(
                "Docker is not reachable. Install Docker Desktop / OrbStack / \
                 colima, or set DOCKER_HOST to a running daemon."
            );
        }
        let image = std::env::var("IRONCLAW_SANDBOX_IMAGE")
            .unwrap_or_else(|_| "ironclaw/sandbox:dev".to_string());
        if !sandbox_image_present(&image).await {
            skip!(
                "Sandbox image '{image}' not found locally. Build it once with:\n\
                 \n    docker build -f crates/Dockerfile.sandbox -t {image} .\n"
            );
        }

        // Replay mode needs a committed trace fixture; live mode needs LLM
        // credentials in `~/.ironclaw/.env`. Skip cleanly when neither is
        // available so a dev running the test for the first time gets a
        // helpful hint instead of a panic deep inside the replay harness.
        let live_mode = std::env::var("IRONCLAW_LIVE_TEST")
            .ok()
            .filter(|v| !v.is_empty() && v != "0")
            .is_some();
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/llm_traces/live/sandbox_clones_ironclaw_to_megaclaw.json");
        if !live_mode && !fixture.exists() {
            skip!(
                "No trace fixture at {} and IRONCLAW_LIVE_TEST is not set. \
                 Record it once with:\n\n    \
                 SANDBOX_ENABLED=true IRONCLAW_LIVE_TEST=1 \\\n    \
                   cargo test --features libsql --test sandbox_live_e2e -- --ignored --nocapture\n\n\
                 (requires LLM credentials in ~/.ironclaw/.env)",
                fixture.display()
            );
        }

        // Force SANDBOX_ENABLED on for this test so the router wires the
        // containerized mount factory. Serialized via ENV_MUTEX so parallel
        // `--ignored` tests don't race on process-wide env mutation.
        //
        // SAFETY: test-only process-wide env mutation; see the Rust 1.80
        // unsafe-env guidance. The mutex prevents concurrent mutation.
        // The guard is dropped before the first `.await` to satisfy clippy.
        static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let prev_sandbox_val = std::env::var("SANDBOX_ENABLED").ok();
        {
            let _env_guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            unsafe {
                std::env::set_var("SANDBOX_ENABLED", "true");
            }
        }

        let harness = LiveTestHarnessBuilder::new("sandbox_clones_ironclaw_to_megaclaw")
            .with_engine_v2(true)
            .with_auto_approve_tools(true)
            .with_max_tool_iterations(60)
            .build()
            .await;

        // The prompt is written to be specific enough that the agent takes
        // deterministic tool actions (git clone, sed, cargo check) while
        // leaving enough room for LLM variation that the test doesn't
        // over-constrain the path. The `/project` prefix matches the
        // sandbox mount the backend bind-mounts from the host workspace.
        let user_input = "\
            You are running inside a sandboxed environment with a writable \
            /project directory that persists on the host. Do the following \
            using the `shell` tool — do NOT use read_file/write_file. \
            Be concise and batch commands when it helps:\n\
            \n\
            1. Clone https://github.com/nearai/ironclaw into /project/repo \
            with a shallow clone: \
            `git clone --depth 1 https://github.com/nearai/ironclaw /project/repo`\n\
            2. Rename the project from 'ironclaw' to 'megaclaw' in the \
            top-level Cargo.toml by updating only the line that reads \
            `name = \"ironclaw\"` to `name = \"megaclaw\"`. Use sed:\n   \
            `sed -i 's/^name = \"ironclaw\"$/name = \"megaclaw\"/' /project/repo/Cargo.toml`\n\
            3. Verify the rename by running \
            `grep -n 'name = \"megaclaw\"' /project/repo/Cargo.toml` and \
            confirm it prints the renamed line.\n\
            \n\
            When the grep succeeds, summarize the three steps you took and \
            include the grep output verbatim as proof. Stop there — no \
            further verification needed.";

        let rig = harness.rig();
        rig.send_message(user_input).await;

        // Wall clock budget: cold container start (~1s) + shallow clone
        // (~10-30s on a fast link) + sed (~0.1s) + grep (~0.1s) + LLM
        // turns (~30-60s). 5 minutes is generous but bounded.
        let responses = rig.wait_for_responses(1, Duration::from_secs(300)).await;
        assert!(!responses.is_empty(), "Expected at least one response");

        let text: Vec<String> = responses.iter().map(|r| r.content.clone()).collect();
        let tools = rig.tool_calls_started();

        eprintln!("[SandboxE2E] Tools used: {tools:?}");
        eprintln!(
            "[SandboxE2E] Response preview: {}",
            text.join("\n").chars().take(600).collect::<String>()
        );

        // Assertion 1: shell ran. Without this the sandbox was never
        // exercised and the test is vacuous. Tool names in the status
        // stream are formatted as `"shell(preview)"` — match by prefix so
        // both forms work.
        assert!(
            tools
                .iter()
                .any(|t| t == "shell" || t.starts_with("shell(")),
            "Expected the shell tool to run inside the sandbox, but no \
             shell call was recorded. Tools: {tools:?}"
        );

        // Assertion 2: the agent's summary mentions megaclaw. This is the
        // cheap proof that it performed (and acknowledged) the rename.
        let joined = text.join("\n").to_lowercase();
        assert!(
            joined.contains("megaclaw"),
            "Agent summary should mention 'megaclaw' but did not. \
             Full response: {joined}"
        );

        // Assertion 3 (live only): LLM judge signs off on all three steps.
        // This catches the case where the agent claimed success but skipped
        // a step, because the judge reads the full response with fresh eyes.
        if let Some(verdict) = harness.judge(&text, JUDGE_CRITERIA).await {
            assert!(
                verdict.pass,
                "LLM judge rejected the scenario: {}",
                verdict.reasoning
            );
        }

        harness.finish(user_input, &text).await;

        // Restore the original env var value so later tests in the same
        // process see the state they started with.
        {
            let _env_guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            unsafe {
                match prev_sandbox_val {
                    Some(v) => std::env::set_var("SANDBOX_ENABLED", v),
                    None => std::env::remove_var("SANDBOX_ENABLED"),
                }
            }
        }
    }
}
