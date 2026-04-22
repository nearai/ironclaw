//! Live/replay A/B benchmarks for CodeAct host-backed shims.
//!
//! These scenarios compare two prompt styles for the same task:
//! - **raw**: force canonical host-tool usage (`read_file`, `write_file`, `glob`, ...)
//! - **shim**: prefer the new Pythonic helpers (`read_json`, `append_text`, `find_files`, ...)
//!
//! The goal is not to make a fragile claim about exact token counts. Instead,
//! these tests verify that both variants succeed end-to-end through engine v2,
//! preserve the same canonical trust boundary, and emit comparable metrics that
//! we can inspect in live mode or replay from committed fixtures.
//!
//! # Running
//!
//! **Replay mode** (deterministic, after fixtures have been recorded):
//! ```bash
//! cargo test --features libsql --test e2e_live_codeact_shims -- --ignored --nocapture
//! ```
//!
//! **Live mode** (real LLM calls, records/updates raw + shim fixtures):
//! ```bash
//! IRONCLAW_LIVE_TEST=1 cargo test --features libsql --test e2e_live_codeact_shims -- --ignored --nocapture --test-threads=1
//! ```

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use std::time::Duration;

    use tokio::sync::Mutex;

    use crate::support::cleanup::CleanupGuard;
    use crate::support::live_harness::{LiveTestHarnessBuilder, TestMode};
    use crate::support::metrics::{RunResult, ScenarioResult, TraceMetrics, compare_runs};

    const ROOT: &str = "/tmp/ironclaw_live_codeact_shims";

    fn engine_v2_live_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[derive(Debug, Clone, Copy)]
    enum Variant {
        Raw,
        Shim,
    }

    impl Variant {
        fn label(self) -> &'static str {
            match self {
                Self::Raw => "raw",
                Self::Shim => "shim",
            }
        }
    }

    #[derive(Debug, Clone)]
    struct VariantRun {
        scenario_id: &'static str,
        variant: Variant,
        trace: TraceMetrics,
        response: String,
        tool_calls_started: Vec<String>,
        tool_calls_completed: Vec<(String, bool)>,
        trace_errors: Vec<String>,
    }

    fn trace_fixture_path(test_name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("llm_traces")
            .join("live")
            .join(format!("{test_name}.json"))
    }

    fn is_live_mode() -> bool {
        std::env::var("IRONCLAW_LIVE_TEST")
            .ok()
            .filter(|v| !v.is_empty() && v != "0")
            .is_some()
    }

    fn variant_test_name(scenario_id: &str, variant: Variant) -> String {
        format!("codeact_host_shims_{scenario_id}_{}", variant.label())
    }

    fn should_run_fixture_pair(scenario_id: &str) -> bool {
        if is_live_mode() {
            return true;
        }

        let raw = variant_test_name(scenario_id, Variant::Raw);
        let shim = variant_test_name(scenario_id, Variant::Shim);
        let raw_path = trace_fixture_path(&raw);
        let shim_path = trace_fixture_path(&shim);
        if raw_path.exists() && shim_path.exists() {
            true
        } else {
            eprintln!(
                "[{scenario_id}] replay fixtures missing; record both variants in live mode first:\n  - {}\n  - {}",
                raw_path.display(),
                shim_path.display()
            );
            false
        }
    }

    fn reset_dir(path: &Path) {
        let _ = fs::remove_dir_all(path);
        fs::create_dir_all(path).expect("failed to create scenario directory");
    }

    fn scenario_result(run: &VariantRun) -> ScenarioResult {
        ScenarioResult {
            scenario_id: run.scenario_id.to_string(),
            passed: true,
            trace: run.trace.clone(),
            response: run.response.clone(),
            error: None,
            turn_metrics: Vec::new(),
        }
    }

    fn total_tokens(trace: &TraceMetrics) -> u32 {
        trace.input_tokens + trace.output_tokens
    }

    fn assert_exact_suffix(response: &str, prefix: &str, expected: &str) {
        let trimmed = response.trim();
        let value = trimmed
            .strip_prefix(prefix)
            .unwrap_or_else(|| {
                panic!("expected response to start with {prefix:?}, got {trimmed:?}")
            })
            .trim();
        assert_eq!(value, expected, "unexpected value for prefix {prefix:?}");
    }

    fn tool_name_matches_prefix(name: &str, expected_prefix: &str) -> bool {
        name == expected_prefix || name.starts_with(expected_prefix)
    }

    fn assert_tool_order_prefixes(started: &[String], expected: &[&str]) {
        let mut search_from = 0usize;
        for tool in expected {
            let found_at = started
                .iter()
                .enumerate()
                .skip(search_from)
                .find(|(_, name)| tool_name_matches_prefix(name, tool))
                .map(|(idx, _)| idx);
            match found_at {
                Some(idx) => search_from = idx + 1,
                None => panic!(
                    "assert_tool_order_prefixes: {tool:?} not found after position {} in {:?}. Expected order: {:?}",
                    search_from, started, expected
                ),
            }
        }
    }

    fn assert_no_non_codeact_failures(completed: &[(String, bool)], trace_errors: &[String]) {
        let failed_host_tools: Vec<&str> = completed
            .iter()
            .filter(|(name, success)| !*success && name != "__codeact__")
            .map(|(name, _)| name.as_str())
            .collect();
        assert!(
            failed_host_tools.is_empty(),
            "expected host tools to succeed; failed host tools: {:?}; all completed: {:?}",
            failed_host_tools,
            completed
        );

        let non_codeact_errors: Vec<&String> = trace_errors
            .iter()
            .filter(|err| !err.contains("__codeact__"))
            .collect();
        assert!(
            non_codeact_errors.is_empty(),
            "expected only CodeAct-layer errors, got non-CodeAct trace errors: {:?}",
            non_codeact_errors
        );
    }

    fn print_pair_report(raw: &VariantRun, shim: &VariantRun) {
        let raw_run = RunResult::from_scenarios(
            format!("{}-raw", raw.scenario_id),
            vec![scenario_result(raw)],
        );
        let shim_run = RunResult::from_scenarios(
            format!("{}-shim", shim.scenario_id),
            vec![scenario_result(shim)],
        );
        let deltas = compare_runs(&raw_run, &shim_run, 0.0);

        eprintln!(
            "[CodeactHostShims][{}] {}:   llm_calls={} total_tokens={} tool_calls={} completed_tools={} turns={} wall={}ms",
            raw.scenario_id,
            raw.variant.label(),
            raw.trace.llm_calls,
            total_tokens(&raw.trace),
            raw.trace.total_tool_calls(),
            raw.tool_calls_completed.len(),
            raw.trace.turns,
            raw.trace.wall_time_ms,
        );
        eprintln!(
            "[CodeactHostShims][{}] {}:  llm_calls={} total_tokens={} tool_calls={} completed_tools={} turns={} wall={}ms",
            shim.scenario_id,
            shim.variant.label(),
            shim.trace.llm_calls,
            total_tokens(&shim.trace),
            shim.trace.total_tool_calls(),
            shim.tool_calls_completed.len(),
            shim.trace.turns,
            shim.trace.wall_time_ms,
        );
        if !raw.trace_errors.is_empty() {
            eprintln!(
                "[CodeactHostShims][{}] raw trace errors: {:?}",
                raw.scenario_id, raw.trace_errors
            );
        }
        if !shim.trace_errors.is_empty() {
            eprintln!(
                "[CodeactHostShims][{}] shim trace errors: {:?}",
                shim.scenario_id, shim.trace_errors
            );
        }
        for delta in deltas {
            eprintln!(
                "[CodeactHostShims][{}] delta {}: baseline={} current={} change={:+.2}%",
                raw.scenario_id,
                delta.metric,
                delta.baseline,
                delta.current,
                delta.delta * 100.0,
            );
        }
    }

    async fn run_variant(
        scenario_id: &'static str,
        variant: Variant,
        prompt: String,
    ) -> VariantRun {
        let test_name = variant_test_name(scenario_id, variant);
        let harness = LiveTestHarnessBuilder::new(&test_name)
            .with_engine_v2(true)
            .with_auto_approve_tools(true)
            .with_allow_local_tools(true)
            .with_max_tool_iterations(40)
            .build()
            .await;

        assert_ne!(
            harness.mode(),
            TestMode::Skipped,
            "{} unexpectedly skipped",
            test_name
        );

        let rig = harness.rig();
        let baseline = rig.wait_for_responses(0, Duration::ZERO).await.len();
        rig.send_message(&prompt).await;
        let responses = rig
            .wait_for_responses(baseline + 1, Duration::from_secs(180))
            .await;
        let new_responses: Vec<String> = responses
            .into_iter()
            .skip(baseline)
            .map(|r| r.content)
            .collect();
        assert!(
            !new_responses.is_empty(),
            "expected at least one response for {scenario_id} {}",
            variant.label()
        );

        let response = new_responses.join("\n");
        let tool_calls_started = rig.tool_calls_started();
        let tool_calls_completed = rig.tool_calls_completed();
        let trace_errors = harness.collect_trace_errors();
        assert_no_non_codeact_failures(&tool_calls_completed, &trace_errors);
        let trace = rig.collect_metrics().await;

        harness.finish(&prompt, &new_responses).await;

        VariantRun {
            scenario_id,
            variant,
            trace,
            response,
            tool_calls_started,
            tool_calls_completed,
            trace_errors,
        }
    }

    fn setup_read_json() -> (CleanupGuard, String) {
        let dir = Path::new(ROOT).join("read_json");
        reset_dir(&dir);
        let file = dir.join("payload.json");
        fs::write(&file, r#"{"answer":42,"label":"host-shims"}"#)
            .expect("failed to seed payload.json");
        (
            CleanupGuard::new().dir(dir.display().to_string()),
            file.display().to_string(),
        )
    }

    fn setup_append_text() -> (CleanupGuard, String) {
        let dir = Path::new(ROOT).join("append_text");
        reset_dir(&dir);
        let file = dir.join("notes.txt");
        fs::write(&file, "alpha").expect("failed to seed notes.txt");
        (
            CleanupGuard::new().dir(dir.display().to_string()),
            file.display().to_string(),
        )
    }

    fn setup_find_files() -> (CleanupGuard, String) {
        let dir = Path::new(ROOT).join("find_files");
        reset_dir(&dir);
        fs::write(dir.join("alpha.md"), "# alpha").expect("failed to seed alpha.md");
        fs::write(dir.join("notes.txt"), "ignore me").expect("failed to seed notes.txt");
        let nested = dir.join("nested");
        fs::create_dir_all(&nested).expect("failed to create nested directory");
        fs::write(nested.join("beta.md"), "# beta").expect("failed to seed beta.md");
        (
            CleanupGuard::new().dir(dir.display().to_string()),
            dir.display().to_string(),
        )
    }

    fn setup_package_json_edit() -> (CleanupGuard, String) {
        let dir = Path::new(ROOT).join("package_json_edit");
        reset_dir(&dir);
        let file = dir.join("package.json");
        let package = serde_json::json!({
            "name": "demo-app",
            "private": true,
            "scripts": {
                "test": "vitest"
            }
        });
        fs::write(
            &file,
            serde_json::to_string_pretty(&package).expect("serialize package.json seed"),
        )
        .expect("failed to seed package.json");
        (
            CleanupGuard::new().dir(dir.display().to_string()),
            file.display().to_string(),
        )
    }

    fn setup_tsconfig_edit() -> (CleanupGuard, String) {
        let dir = Path::new(ROOT).join("tsconfig_edit");
        reset_dir(&dir);
        let file = dir.join("tsconfig.json");
        let config = serde_json::json!({
            "compilerOptions": {
                "target": "ES2022",
                "module": "ESNext",
                "strict": false
            },
            "include": ["src"]
        });
        fs::write(
            &file,
            serde_json::to_string_pretty(&config).expect("serialize tsconfig seed"),
        )
        .expect("failed to seed tsconfig.json");
        (
            CleanupGuard::new().dir(dir.display().to_string()),
            file.display().to_string(),
        )
    }

    fn assert_package_json_scripts(path: &str, expected_keys: &[&str]) {
        let content = fs::read_to_string(path).expect("package.json should exist after edit");
        let value: serde_json::Value =
            serde_json::from_str(&content).expect("package.json should contain valid JSON");
        let scripts = value
            .get("scripts")
            .and_then(|v| v.as_object())
            .expect("package.json should contain a scripts object");
        let mut keys: Vec<&str> = scripts.keys().map(|k| k.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(keys, expected_keys, "unexpected package.json scripts set");
        assert_eq!(
            scripts.get("lint").and_then(|v| v.as_str()),
            Some("eslint ."),
            "expected lint script to be added"
        );
        assert_eq!(
            scripts.get("test").and_then(|v| v.as_str()),
            Some("vitest"),
            "expected existing test script to be preserved"
        );
    }

    fn assert_tsconfig_flags(path: &str) {
        let content = fs::read_to_string(path).expect("tsconfig.json should exist after edit");
        let value: serde_json::Value =
            serde_json::from_str(&content).expect("tsconfig.json should contain valid JSON");
        let compiler_options = value
            .get("compilerOptions")
            .and_then(|v| v.as_object())
            .expect("tsconfig.json should contain compilerOptions");
        assert_eq!(
            compiler_options.get("strict").and_then(|v| v.as_bool()),
            Some(true),
            "expected strict to be true"
        );
        assert_eq!(
            compiler_options
                .get("noUncheckedIndexedAccess")
                .and_then(|v| v.as_bool()),
            Some(true),
            "expected noUncheckedIndexedAccess to be true"
        );
        assert_eq!(
            compiler_options.get("target").and_then(|v| v.as_str()),
            Some("ES2022"),
            "expected target to be preserved"
        );
        assert_eq!(
            compiler_options.get("module").and_then(|v| v.as_str()),
            Some("ESNext"),
            "expected module to be preserved"
        );
        let include = value
            .get("include")
            .and_then(|v| v.as_array())
            .expect("expected include array to be preserved");
        assert_eq!(include.len(), 1, "expected include array length to remain 1");
        assert_eq!(include[0].as_str(), Some("src"));
    }

    fn read_json_prompt(path: &str, variant: Variant) -> String {
        match variant {
            Variant::Raw => format!(
                "Use a Python CodeAct script, not plain prose. Read the JSON file at {path:?}, extract the integer field `answer`, and call FINAL exactly with `answer=<value>`. IMPORTANT: this is the RAW variant, so do not use helper shims like read_json, read_text, write_text, append_text, list_entries, find_files, http_get, or run. Use the canonical host tools directly from Python. The canonical read_file tool returns a dict whose `content` field contains numbered text in a format like `     1│ {{\"answer\": 42}}`; strip everything through the `│` separator before parsing."
            ),
            Variant::Shim => format!(
                "Use a Python CodeAct script, not plain prose. Read the JSON file at {path:?}, extract the integer field `answer`, and call FINAL exactly with `answer=<value>`. IMPORTANT: this is the SHIM variant, so prefer read_json(path) instead of manual read_file parsing unless the shim fails."
            ),
        }
    }

    fn append_text_prompt(path: &str, variant: Variant) -> String {
        match variant {
            Variant::Raw => format!(
                "Use a Python CodeAct script, not plain prose. Append the text `beta` to the file at {path:?}, then read the file back and call FINAL exactly with `text=alphabeta`. The existing file already contains exactly `alpha`. IMPORTANT: this is the RAW variant, so do not use helper shims like append_text or read_text. Use canonical host tools directly from Python, preserve the existing `alpha` text, and remember that read_file returns a dict whose `content` field contains numbered text in a format like `     1│ alpha`; strip everything through the `│` separator before joining or rewriting the text."
            ),
            Variant::Shim => format!(
                "Use a Python CodeAct script, not plain prose. Append the text `beta` to the file at {path:?}, then read the file back and call FINAL exactly with `text=alphabeta`. The existing file already contains exactly `alpha`. IMPORTANT: this is the SHIM variant, so prefer append_text(path, text) and read_text(path) unless they fail. `append_text` concatenates raw text directly, so pass plain `beta` here."
            ),
        }
    }

    fn find_files_prompt(path: &str, variant: Variant) -> String {
        match variant {
            Variant::Raw => format!(
                "Use a Python CodeAct script, not plain prose. Under directory {path:?}, find markdown files recursively and return their basenames sorted alphabetically. Then call FINAL exactly with `matches=alpha.md,beta.md`. IMPORTANT: this is the RAW variant, so do not use helper shims like find_files or list_entries. Use the canonical glob tool directly from Python. The canonical glob tool returns a dict with a `files` list of relative paths. Do not import os or os.path; derive basenames by splitting each relative path on '/'."
            ),
            Variant::Shim => format!(
                "Use a Python CodeAct script, not plain prose. Under directory {path:?}, find markdown files recursively and return their basenames sorted alphabetically. Then call FINAL exactly with `matches=alpha.md,beta.md`. IMPORTANT: this is the SHIM variant, so prefer find_files(pattern, path=...) unless it fails. The shim returns the matched file paths directly, so split each returned path on '/'."
            ),
        }
    }

    fn package_json_edit_prompt(path: &str, variant: Variant) -> String {
        match variant {
            Variant::Raw => format!(
                "Use a Python CodeAct script, not plain prose. Edit the repo file {path:?} as if you were updating a real package.json. Add a new script entry `lint: eslint .` under `scripts`, preserve the existing `test: vitest` script, write the updated file back as pretty JSON, then re-read the saved file and call FINAL exactly with `scripts=lint,test`. IMPORTANT: this is the RAW variant, so do not use helper shims like read_json, write_json, read_text, or write_text. Use canonical host tools directly from Python. The canonical read_file tool returns a dict whose `content` field contains numbered text in a format like `     1│ {{...}}`; strip everything through the `│` separator before JSON parsing. After writing, derive the final `scripts=...` string from the actual saved file by sorting the script keys alphabetically."
            ),
            Variant::Shim => format!(
                "Use a Python CodeAct script, not plain prose. Edit the repo file {path:?} as if you were updating a real package.json. Add a new script entry `lint: eslint .` under `scripts`, preserve the existing `test: vitest` script, write the updated file back as pretty JSON, then re-read the saved file and call FINAL exactly with `scripts=lint,test`. IMPORTANT: this is the SHIM variant, so prefer read_json(path) and write_json(path, value). In CodeAct these helpers are async, so use `await read_json(...)` and `await write_json(...)`. After writing, re-read the actual file and derive the final `scripts=...` string by sorting the script keys alphabetically."
            ),
        }
    }

    fn tsconfig_edit_prompt(path: &str, variant: Variant) -> String {
        match variant {
            Variant::Raw => format!(
                "Use a Python CodeAct script, not plain prose. Edit the repo file {path:?} as if you were updating a real tsconfig.json. Inside `compilerOptions`, set `strict` to true and add `noUncheckedIndexedAccess: true`, while preserving the existing `target`, `module`, and `include` entries. Write the updated file back as pretty JSON, then re-read the saved file and call FINAL exactly with `flags=noUncheckedIndexedAccess:true,strict:true`. IMPORTANT: this is the RAW variant, so do not use helper shims like read_json, write_json, read_text, or write_text. Use canonical host tools directly from Python. The canonical read_file tool returns a dict whose `content` field contains numbered text in a format like `     1│ {{...}}`; strip everything through the `│` separator before JSON parsing. After writing, derive the final flags string from the actual saved file."
            ),
            Variant::Shim => format!(
                "Use a Python CodeAct script, not plain prose. Edit the repo file {path:?} as if you were updating a real tsconfig.json. Inside `compilerOptions`, set `strict` to true and add `noUncheckedIndexedAccess: true`, while preserving the existing `target`, `module`, and `include` entries. Write the updated file back as pretty JSON, then re-read the saved file and call FINAL exactly with `flags=noUncheckedIndexedAccess:true,strict:true`. IMPORTANT: this is the SHIM variant, so prefer `await read_json(path)` and `await write_json(path, value)`. After writing, re-read the actual file and derive the final flags string from the saved file."
            ),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn codeact_read_json_raw_vs_shim() {
        let _guard = engine_v2_live_lock().lock().await;
        if !should_run_fixture_pair("read_json") {
            return;
        }

        let (_cleanup, path) = setup_read_json();
        let raw = run_variant(
            "read_json",
            Variant::Raw,
            read_json_prompt(&path, Variant::Raw),
        )
        .await;
        assert_exact_suffix(&raw.response, "answer=", "42");
        assert!(
            !raw.tool_calls_completed.is_empty(),
            "raw variant should execute at least one tool call"
        );
        assert!(
            raw.tool_calls_started
                .iter()
                .any(|name| name.starts_with("read_file")),
            "raw variant should use read_file, got {:?}",
            raw.tool_calls_started
        );

        let (_cleanup, path) = setup_read_json();
        let shim = run_variant(
            "read_json",
            Variant::Shim,
            read_json_prompt(&path, Variant::Shim),
        )
        .await;
        assert_exact_suffix(&shim.response, "answer=", "42");
        assert!(
            !shim.tool_calls_completed.is_empty(),
            "shim variant should execute at least one tool call"
        );
        assert!(
            shim.tool_calls_started
                .iter()
                .any(|name| name.starts_with("read_file")),
            "shim variant should still hit canonical read_file, got {:?}",
            shim.tool_calls_started
        );

        print_pair_report(&raw, &shim);
    }

    #[tokio::test]
    #[ignore]
    async fn codeact_append_text_raw_vs_shim() {
        let _guard = engine_v2_live_lock().lock().await;
        if !should_run_fixture_pair("append_text") {
            return;
        }

        let (_cleanup, path) = setup_append_text();
        let raw = run_variant(
            "append_text",
            Variant::Raw,
            append_text_prompt(&path, Variant::Raw),
        )
        .await;
        assert_exact_suffix(&raw.response, "text=", "alphabeta");
        assert!(
            !raw.tool_calls_completed.is_empty(),
            "raw append_text variant should execute tool calls"
        );
        assert_tool_order_prefixes(
            &raw.tool_calls_started,
            &["read_file", "write_file", "read_file"],
        );
        let raw_content =
            fs::read_to_string(&path).expect("raw append_text should update notes.txt");
        assert_eq!(
            raw_content, "alphabeta",
            "raw variant wrote unexpected content"
        );

        let (_cleanup, path) = setup_append_text();
        let shim = run_variant(
            "append_text",
            Variant::Shim,
            append_text_prompt(&path, Variant::Shim),
        )
        .await;
        assert_exact_suffix(&shim.response, "text=", "alphabeta");
        assert!(
            !shim.tool_calls_completed.is_empty(),
            "shim append_text variant should execute tool calls"
        );
        assert_tool_order_prefixes(
            &shim.tool_calls_started,
            &["read_file", "write_file", "read_file"],
        );
        let shim_content =
            fs::read_to_string(&path).expect("shim append_text should update notes.txt");
        assert_eq!(
            shim_content, "alphabeta",
            "shim variant wrote unexpected content"
        );

        print_pair_report(&raw, &shim);
    }

    #[tokio::test]
    #[ignore]
    async fn codeact_find_files_raw_vs_shim() {
        let _guard = engine_v2_live_lock().lock().await;
        if !should_run_fixture_pair("find_files") {
            return;
        }

        let (_cleanup, path) = setup_find_files();
        let raw = run_variant(
            "find_files",
            Variant::Raw,
            find_files_prompt(&path, Variant::Raw),
        )
        .await;
        assert_exact_suffix(&raw.response, "matches=", "alpha.md,beta.md");
        assert!(
            !raw.tool_calls_completed.is_empty(),
            "raw find_files variant should execute tool calls"
        );
        assert!(
            raw.tool_calls_started
                .iter()
                .any(|name| name.starts_with("glob")),
            "raw variant should use glob, got {:?}",
            raw.tool_calls_started
        );

        let (_cleanup, path) = setup_find_files();
        let shim = run_variant(
            "find_files",
            Variant::Shim,
            find_files_prompt(&path, Variant::Shim),
        )
        .await;
        assert_exact_suffix(&shim.response, "matches=", "alpha.md,beta.md");
        assert!(
            !shim.tool_calls_completed.is_empty(),
            "shim find_files variant should execute tool calls"
        );
        assert!(
            shim.tool_calls_started
                .iter()
                .any(|name| name.starts_with("glob")),
            "shim variant should still hit canonical glob, got {:?}",
            shim.tool_calls_started
        );

        print_pair_report(&raw, &shim);
    }

    #[tokio::test]
    #[ignore]
    async fn codeact_package_json_edit_raw_vs_shim() {
        let _guard = engine_v2_live_lock().lock().await;
        if !should_run_fixture_pair("package_json_edit") {
            return;
        }

        let (_cleanup, path) = setup_package_json_edit();
        let raw = run_variant(
            "package_json_edit",
            Variant::Raw,
            package_json_edit_prompt(&path, Variant::Raw),
        )
        .await;
        assert_exact_suffix(&raw.response, "scripts=", "lint,test");
        assert!(
            raw.tool_calls_started
                .iter()
                .any(|name| name.starts_with("read_file")),
            "raw package.json variant should use read_file, got {:?}",
            raw.tool_calls_started
        );
        assert!(
            raw.tool_calls_started
                .iter()
                .any(|name| name.starts_with("write_file")),
            "raw package.json variant should use write_file, got {:?}",
            raw.tool_calls_started
        );
        assert_package_json_scripts(&path, &["lint", "test"]);

        let (_cleanup, path) = setup_package_json_edit();
        let shim = run_variant(
            "package_json_edit",
            Variant::Shim,
            package_json_edit_prompt(&path, Variant::Shim),
        )
        .await;
        assert_exact_suffix(&shim.response, "scripts=", "lint,test");
        assert!(
            shim.tool_calls_started
                .iter()
                .any(|name| name.starts_with("read_file")),
            "shim package.json variant should still hit canonical read_file, got {:?}",
            shim.tool_calls_started
        );
        assert!(
            shim.tool_calls_started
                .iter()
                .any(|name| name.starts_with("write_file")),
            "shim package.json variant should still hit canonical write_file, got {:?}",
            shim.tool_calls_started
        );
        assert_package_json_scripts(&path, &["lint", "test"]);

        print_pair_report(&raw, &shim);
    }

    #[tokio::test]
    #[ignore]
    async fn codeact_tsconfig_edit_raw_vs_shim() {
        let _guard = engine_v2_live_lock().lock().await;
        if !should_run_fixture_pair("tsconfig_edit") {
            return;
        }

        let (_cleanup, path) = setup_tsconfig_edit();
        let raw = run_variant(
            "tsconfig_edit",
            Variant::Raw,
            tsconfig_edit_prompt(&path, Variant::Raw),
        )
        .await;
        assert_exact_suffix(
            &raw.response,
            "flags=",
            "noUncheckedIndexedAccess:true,strict:true",
        );
        assert!(
            raw.tool_calls_started
                .iter()
                .any(|name| name.starts_with("read_file")),
            "raw tsconfig variant should use read_file, got {:?}",
            raw.tool_calls_started
        );
        assert!(
            raw.tool_calls_started
                .iter()
                .any(|name| name.starts_with("write_file")),
            "raw tsconfig variant should use write_file, got {:?}",
            raw.tool_calls_started
        );
        assert_tsconfig_flags(&path);

        let (_cleanup, path) = setup_tsconfig_edit();
        let shim = run_variant(
            "tsconfig_edit",
            Variant::Shim,
            tsconfig_edit_prompt(&path, Variant::Shim),
        )
        .await;
        assert_exact_suffix(
            &shim.response,
            "flags=",
            "noUncheckedIndexedAccess:true,strict:true",
        );
        assert!(
            shim.tool_calls_started
                .iter()
                .any(|name| name.starts_with("read_file")),
            "shim tsconfig variant should still hit canonical read_file, got {:?}",
            shim.tool_calls_started
        );
        assert!(
            shim.tool_calls_started
                .iter()
                .any(|name| name.starts_with("write_file")),
            "shim tsconfig variant should still hit canonical write_file, got {:?}",
            shim.tool_calls_started
        );
        assert_tsconfig_flags(&path);

        print_pair_report(&raw, &shim);
    }
}
