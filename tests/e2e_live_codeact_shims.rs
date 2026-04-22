//! Live/replay A/B benchmarks for CodeAct host-backed shims.
//!
//! These scenarios compare paired prompt styles for the same task.
//! Most scenarios are:
//! - **raw**: force canonical host-tool usage (`read_file`, `write_file`, `glob`, ...)
//! - **shim**: prefer the new Pythonic helpers (`read_json`, `append_text`, `find_files`, ...)
//!
//! A smaller set can also compare:
//! - **shim**: current normalized dict-like shim results
//! - **rich**: experimental host-backed rich result objects (`HttpResponse`, `CompletedProcess`)
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

    use axum::{
        Json, Router,
        extract::State,
        http::StatusCode,
        routing::{get, post},
    };
    use tokio::net::TcpListener;
    use tokio::sync::{Mutex, oneshot};

    use crate::support::cleanup::CleanupGuard;
    use crate::support::live_harness::{LiveTestHarnessBuilder, TestMode};
    use crate::support::metrics::{RunResult, ScenarioResult, TraceMetrics, compare_runs};
    use crate::support::trace_llm::{LlmTrace, TraceResponse};

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
        variant: &'static str,
        trace: TraceMetrics,
        response: String,
        tool_calls_started: Vec<String>,
        tool_calls_completed: Vec<(String, bool)>,
        trace_errors: Vec<String>,
    }

    struct HttpScenarioGuard {
        _cleanup: CleanupGuard,
        shutdown: Option<oneshot::Sender<()>>,
    }

    impl Drop for HttpScenarioGuard {
        fn drop(&mut self) {
            if let Some(tx) = self.shutdown.take() {
                let _ = tx.send(());
            }
        }
    }

    #[derive(Clone)]
    struct HttpResponseScenarioState {
        required_status: String,
        required_marker: String,
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

    fn variant_test_name_for_label(scenario_id: &str, label: &str) -> String {
        format!("codeact_host_shims_{scenario_id}_{label}")
    }

    fn should_run_fixture_names(scenario_id: &str, labels: &[&str]) -> bool {
        if is_live_mode() {
            return true;
        }

        let missing: Vec<PathBuf> = labels
            .iter()
            .map(|label| trace_fixture_path(&variant_test_name_for_label(scenario_id, label)))
            .filter(|path| !path.exists())
            .collect();

        if missing.is_empty() {
            true
        } else {
            eprintln!(
                "[{scenario_id}] replay fixtures missing; record these variants in live mode first:"
            );
            for path in missing {
                eprintln!("  - {}", path.display());
            }
            false
        }
    }

    fn should_run_fixture_pair(scenario_id: &str) -> bool {
        should_run_fixture_names(scenario_id, &[Variant::Raw.label(), Variant::Shim.label()])
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

    fn recorded_text_response(scenario_id: &str, label: &str) -> String {
        let trace = LlmTrace::from_file(trace_fixture_path(&variant_test_name_for_label(
            scenario_id,
            label,
        )))
        .unwrap_or_else(|e| panic!("failed to load recorded trace for {scenario_id}/{label}: {e}"));

        let parts: Vec<String> = trace
            .turns
            .iter()
            .flat_map(|turn| turn.steps.iter())
            .filter_map(|step| match &step.response {
                TraceResponse::Text { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect();

        assert!(
            !parts.is_empty(),
            "missing recorded text response for {scenario_id}/{label}"
        );
        parts.join("\n")
    }

    async fn http_plan_handler(
        State(state): State<HttpResponseScenarioState>,
    ) -> Json<serde_json::Value> {
        Json(serde_json::json!({
            "required_status": state.required_status,
            "required_marker": state.required_marker,
        }))
    }

    async fn http_validate_handler(
        State(state): State<HttpResponseScenarioState>,
        Json(payload): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let content = payload
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        let mut missing = Vec::new();
        let required_status_line = format!("status={}", state.required_status);
        if !content
            .lines()
            .any(|line| line.trim() == required_status_line)
        {
            missing.push(required_status_line);
        }
        if !content
            .lines()
            .any(|line| line.trim() == state.required_marker)
        {
            missing.push(state.required_marker.clone());
        }

        if missing.is_empty() {
            (
                StatusCode::OK,
                Json(serde_json::json!({ "ok": true, "summary": "validated" })),
            )
        } else {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({ "ok": false, "missing": missing })),
            )
        }
    }

    async fn start_http_response_result_object_server() -> (String, oneshot::Sender<()>) {
        let state = HttpResponseScenarioState {
            required_status: "done".to_string(),
            required_marker: "reviewed=true".to_string(),
        };
        let app = Router::new()
            .route("/plan", get(http_plan_handler))
            .route("/validate", post(http_validate_handler))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local http scenario server");
        let addr = listener.local_addr().expect("local http scenario addr");
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await;
        });
        (format!("http://{addr}"), shutdown_tx)
    }

    fn ensure_http_allow_localhost_enabled() {
        static INIT: OnceLock<()> = OnceLock::new();
        INIT.get_or_init(|| {
            // SAFETY: this ignored live/replay benchmark binary is run serially,
            // and HttpTool caches this flag in a OnceLock on first use. We set
            // it once process-wide so localhost test servers remain available
            // regardless of test execution order.
            unsafe {
                std::env::set_var("HTTP_ALLOW_LOCALHOST", "1");
            }
        });
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

    fn print_pair_report(baseline: &VariantRun, candidate: &VariantRun) {
        let baseline_run = RunResult::from_scenarios(
            format!("{}-{}", baseline.scenario_id, baseline.variant),
            vec![scenario_result(baseline)],
        );
        let candidate_run = RunResult::from_scenarios(
            format!("{}-{}", candidate.scenario_id, candidate.variant),
            vec![scenario_result(candidate)],
        );
        let deltas = compare_runs(&baseline_run, &candidate_run, 0.0);

        eprintln!(
            "[CodeactHostShims][{}] {}:   llm_calls={} total_tokens={} tool_calls={} completed_tools={} turns={} wall={}ms",
            baseline.scenario_id,
            baseline.variant,
            baseline.trace.llm_calls,
            total_tokens(&baseline.trace),
            baseline.trace.total_tool_calls(),
            baseline.tool_calls_completed.len(),
            baseline.trace.turns,
            baseline.trace.wall_time_ms,
        );
        eprintln!(
            "[CodeactHostShims][{}] {}:  llm_calls={} total_tokens={} tool_calls={} completed_tools={} turns={} wall={}ms",
            candidate.scenario_id,
            candidate.variant,
            candidate.trace.llm_calls,
            total_tokens(&candidate.trace),
            candidate.trace.total_tool_calls(),
            candidate.tool_calls_completed.len(),
            candidate.trace.turns,
            candidate.trace.wall_time_ms,
        );
        if !baseline.trace_errors.is_empty() {
            eprintln!(
                "[CodeactHostShims][{}] {} trace errors: {:?}",
                baseline.scenario_id, baseline.variant, baseline.trace_errors
            );
        }
        if !candidate.trace_errors.is_empty() {
            eprintln!(
                "[CodeactHostShims][{}] {} trace errors: {:?}",
                candidate.scenario_id, candidate.variant, candidate.trace_errors
            );
        }
        for delta in deltas {
            eprintln!(
                "[CodeactHostShims][{}] delta {}: baseline={} current={} change={:+.2}%",
                baseline.scenario_id,
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
        run_named_variant_with(
            scenario_id,
            variant.label(),
            prompt,
            /* tolerate_host_failures */ false,
            /* rich_result_objects */ false,
        )
        .await
    }

    async fn run_variant_tolerant(
        scenario_id: &'static str,
        variant: Variant,
        prompt: String,
    ) -> VariantRun {
        run_named_variant_with(
            scenario_id,
            variant.label(),
            prompt,
            /* tolerate_host_failures */ true,
            /* rich_result_objects */ false,
        )
        .await
    }

    async fn run_named_variant(
        scenario_id: &'static str,
        label: &'static str,
        prompt: String,
        rich_result_objects: bool,
    ) -> VariantRun {
        run_named_variant_with(
            scenario_id,
            label,
            prompt,
            /* tolerate_host_failures */ false,
            rich_result_objects,
        )
        .await
    }

    async fn run_named_variant_with(
        scenario_id: &'static str,
        label: &'static str,
        prompt: String,
        tolerate_host_failures: bool,
        rich_result_objects: bool,
    ) -> VariantRun {
        ensure_http_allow_localhost_enabled();

        let test_name = variant_test_name_for_label(scenario_id, label);
        let harness = LiveTestHarnessBuilder::new(&test_name)
            .with_engine_v2(true)
            .with_auto_approve_tools(true)
            .with_allow_local_tools(true)
            .with_codeact_host_shims(true)
            .with_codeact_host_result_objects(rich_result_objects)
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
            label
        );

        let response = new_responses.join("\n");
        let tool_calls_started = rig.tool_calls_started();
        let tool_calls_completed = rig.tool_calls_completed();
        let trace_errors = harness.collect_trace_errors();
        if !tolerate_host_failures {
            assert_no_non_codeact_failures(&tool_calls_completed, &trace_errors);
        }
        let trace = rig.collect_metrics().await;

        harness.finish(&prompt, &new_responses).await;

        VariantRun {
            scenario_id,
            variant: label,
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

    fn setup_completed_process_result_object(label: &str) -> (CleanupGuard, String) {
        let dir = Path::new(ROOT)
            .join("completed_process_result_object")
            .join(label);
        reset_dir(&dir);
        let script = dir.join("emit_lines.sh");
        fs::write(&script, "#!/bin/sh\nprintf 'alpha\\nbeta\\n'")
            .expect("failed to seed emit_lines.sh");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script)
                .expect("failed to stat emit_lines.sh")
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script, perms).expect("failed to chmod emit_lines.sh");
        }
        (
            CleanupGuard::new().dir(dir.display().to_string()),
            dir.display().to_string(),
        )
    }

    fn setup_completed_process_recovery(label: &str) -> (CleanupGuard, String) {
        let dir = Path::new(ROOT)
            .join("completed_process_recovery")
            .join(label);
        reset_dir(&dir);
        let config = dir.join("app.conf");
        fs::write(&config, "name=demo\n").expect("failed to seed app.conf");
        let check = dir.join("check_config.sh");
        fs::write(
            &check,
            "#!/bin/sh\nif grep -qx 'mode=enabled' app.conf; then\n  printf 'config ok\\n'\n  exit 0\nelse\n  printf 'missing mode=enabled in app.conf\\n' >&2\n  exit 7\nfi\n",
        )
        .expect("failed to seed check_config.sh");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&check)
                .expect("failed to stat check_config.sh")
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&check, perms).expect("failed to chmod check_config.sh");
        }
        (
            CleanupGuard::new().dir(dir.display().to_string()),
            dir.display().to_string(),
        )
    }

    async fn setup_http_response_result_object(label: &str) -> (HttpScenarioGuard, String, String) {
        let dir = Path::new(ROOT)
            .join("http_response_result_object")
            .join(label);
        reset_dir(&dir);
        let status_file = dir.join("status.txt");
        fs::write(&status_file, "status=pending\nowner=alice\n")
            .expect("failed to seed status.txt");
        let (base_url, shutdown) = start_http_response_result_object_server().await;
        (
            HttpScenarioGuard {
                _cleanup: CleanupGuard::new().dir(dir.display().to_string()),
                shutdown: Some(shutdown),
            },
            dir.display().to_string(),
            base_url,
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

    fn setup_monorepo_package_migration() -> (CleanupGuard, String) {
        let dir = Path::new(ROOT).join("monorepo_package_migration");
        reset_dir(&dir);
        let packages = dir.join("packages");
        fs::create_dir_all(&packages).expect("failed to create packages dir");

        let seed = [
            ("app", serde_json::json!({"test": "vitest"})),
            ("admin", serde_json::json!({"build": "tsup"})),
            ("docs", serde_json::json!({"preview": "vitepress preview"})),
        ];
        for (name, scripts) in seed {
            let pkg_dir = packages.join(name);
            fs::create_dir_all(&pkg_dir).expect("failed to create package dir");
            let pkg = serde_json::json!({
                "name": format!("@demo/{name}"),
                "private": true,
                "scripts": scripts,
            });
            fs::write(
                pkg_dir.join("package.json"),
                serde_json::to_string_pretty(&pkg).expect("serialize monorepo package.json"),
            )
            .expect("failed to seed monorepo package.json");
        }
        (
            CleanupGuard::new().dir(dir.display().to_string()),
            dir.display().to_string(),
        )
    }

    fn assert_monorepo_package_migration(root: &str) {
        let packages = Path::new(root).join("packages");
        let expected: &[(&str, &[(&str, &str)])] = &[
            ("admin", &[("build", "tsup"), ("lint", "eslint .")]),
            ("app", &[("lint", "eslint ."), ("test", "vitest")]),
            (
                "docs",
                &[("lint", "eslint ."), ("preview", "vitepress preview")],
            ),
        ];
        for (name, wanted) in expected {
            let path = packages.join(name).join("package.json");
            let content = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("missing package.json for {name}: {e}"));
            let value: serde_json::Value = serde_json::from_str(&content)
                .unwrap_or_else(|e| panic!("invalid JSON for {name}: {e}"));
            let scripts = value
                .get("scripts")
                .and_then(|v| v.as_object())
                .unwrap_or_else(|| panic!("package.json for {name} missing scripts object"));
            let mut actual: Vec<(&str, &str)> = scripts
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str().unwrap_or("")))
                .collect();
            actual.sort_unstable_by(|a, b| a.0.cmp(b.0));
            assert_eq!(actual, *wanted, "unexpected scripts for {name}");
            assert_eq!(
                value.get("name").and_then(|v| v.as_str()),
                Some(format!("@demo/{name}").as_str()),
                "expected name to be preserved for {name}",
            );
            assert_eq!(
                value.get("private").and_then(|v| v.as_bool()),
                Some(true),
                "expected private flag to be preserved for {name}",
            );
        }
    }

    fn setup_tsconfig_nested_paths() -> (CleanupGuard, String) {
        let dir = Path::new(ROOT).join("tsconfig_nested_paths");
        reset_dir(&dir);
        let file = dir.join("tsconfig.json");
        let config = serde_json::json!({
            "compilerOptions": {
                "target": "ES2022",
                "module": "ESNext",
                "strict": false,
                "paths": {
                    "@app/*": ["src/app/*"],
                    "@lib/*": ["src/lib/*"]
                }
            },
            "include": ["src"],
            "exclude": ["dist"]
        });
        fs::write(
            &file,
            serde_json::to_string_pretty(&config).expect("serialize nested tsconfig seed"),
        )
        .expect("failed to seed nested tsconfig.json");
        (
            CleanupGuard::new().dir(dir.display().to_string()),
            file.display().to_string(),
        )
    }

    fn assert_tsconfig_nested_paths(path: &str) {
        let content =
            fs::read_to_string(path).expect("nested tsconfig.json should exist after edit");
        let value: serde_json::Value =
            serde_json::from_str(&content).expect("nested tsconfig.json should contain valid JSON");
        let compiler_options = value
            .get("compilerOptions")
            .and_then(|v| v.as_object())
            .expect("nested tsconfig.json should contain compilerOptions");
        assert_eq!(
            compiler_options.get("strict").and_then(|v| v.as_bool()),
            Some(true),
            "expected strict to be true",
        );
        assert_eq!(
            compiler_options
                .get("noUncheckedIndexedAccess")
                .and_then(|v| v.as_bool()),
            Some(true),
            "expected noUncheckedIndexedAccess to be true",
        );
        assert_eq!(
            compiler_options.get("target").and_then(|v| v.as_str()),
            Some("ES2022"),
        );
        assert_eq!(
            compiler_options.get("module").and_then(|v| v.as_str()),
            Some("ESNext"),
        );
        let paths = compiler_options
            .get("paths")
            .and_then(|v| v.as_object())
            .expect("compilerOptions.paths object should be preserved");
        assert_eq!(
            paths
                .get("@app/*")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>()),
            Some(vec!["src/app/*"]),
        );
        assert_eq!(
            paths
                .get("@lib/*")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>()),
            Some(vec!["src/lib/*"]),
        );
        assert_eq!(
            paths
                .get("@ui/*")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>()),
            Some(vec!["src/ui/*"]),
            "expected new @ui/* path mapping to be added",
        );
        assert_eq!(
            value
                .get("include")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first().and_then(|v| v.as_str())),
            Some("src"),
        );
        assert_eq!(
            value
                .get("exclude")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first().and_then(|v| v.as_str())),
            Some("dist"),
        );
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
        assert_eq!(
            include.len(),
            1,
            "expected include array length to remain 1"
        );
        assert_eq!(include[0].as_str(), Some("src"));
    }

    fn setup_js_codemod_use_strict(variant: Variant) -> (CleanupGuard, String) {
        let dir = Path::new(ROOT).join(format!("js_codemod_use_strict_{}", variant.label()));
        reset_dir(&dir);
        let src = dir.join("src");
        fs::create_dir_all(&src).expect("failed to create src dir");
        fs::write(src.join("a.js"), "export const a = 1;\n").expect("seed a.js");
        fs::write(src.join("b.js"), "export const b = 2;\n").expect("seed b.js");
        fs::write(src.join("c.js"), "export const c = 3;\n").expect("seed c.js");
        let sub = src.join("sub");
        fs::create_dir_all(&sub).expect("failed to create sub dir");
        fs::write(sub.join("d.js"), "export const d = 4;\n").expect("seed sub/d.js");
        (
            CleanupGuard::new().dir(dir.display().to_string()),
            dir.display().to_string(),
        )
    }

    fn assert_js_codemod_use_strict(root: &str) {
        let src = Path::new(root).join("src");
        let cases: &[(&str, &str)] = &[
            ("a.js", "export const a = 1"),
            ("b.js", "export const b = 2"),
            ("c.js", "export const c = 3"),
            ("sub/d.js", "export const d = 4"),
        ];
        for (rel, must_contain_decl) in cases {
            let path = src.join(rel);
            let actual =
                fs::read_to_string(&path).unwrap_or_else(|e| panic!("missing js file {rel}: {e}"));
            assert!(
                actual.starts_with("'use strict';\n"),
                "expected {rel} to start with `'use strict';` pragma; got: {actual:?}"
            );
            assert_eq!(
                actual.matches("'use strict';").count(),
                1,
                "expected exactly one `'use strict';` occurrence in {rel}; got: {actual:?}"
            );
            assert!(
                actual.contains(must_contain_decl),
                "expected {rel} to preserve original declaration `{must_contain_decl}`; got: {actual:?}"
            );
        }
    }

    fn setup_mixed_config_sync(variant: Variant) -> (CleanupGuard, String) {
        let dir = Path::new(ROOT).join(format!("mixed_config_sync_{}", variant.label()));
        reset_dir(&dir);
        let services = dir.join("services");
        fs::create_dir_all(&services).expect("failed to create services dir");
        for name in ["api", "worker", "web"] {
            let svc = services.join(name);
            fs::create_dir_all(&svc).expect("failed to create service dir");
            let pkg = serde_json::json!({
                "name": format!("@svc/{name}"),
                "private": true,
                "version": "0.1.0",
            });
            fs::write(
                svc.join("package.json"),
                serde_json::to_string_pretty(&pkg).expect("serialize svc package.json"),
            )
            .expect("seed svc package.json");
            let tsc = serde_json::json!({
                "compilerOptions": {
                    "module": "ESNext",
                    "strict": true,
                },
                "include": ["src"],
            });
            fs::write(
                svc.join("tsconfig.json"),
                serde_json::to_string_pretty(&tsc).expect("serialize svc tsconfig.json"),
            )
            .expect("seed svc tsconfig.json");
        }
        (
            CleanupGuard::new().dir(dir.display().to_string()),
            dir.display().to_string(),
        )
    }

    fn assert_mixed_config_sync(root: &str) {
        let services = Path::new(root).join("services");
        for name in ["api", "web", "worker"] {
            let svc = services.join(name);
            let pkg_content = fs::read_to_string(svc.join("package.json"))
                .unwrap_or_else(|e| panic!("missing package.json for {name}: {e}"));
            let pkg: serde_json::Value = serde_json::from_str(&pkg_content)
                .unwrap_or_else(|e| panic!("invalid package.json for {name}: {e}"));
            assert_eq!(
                pkg.get("name").and_then(|v| v.as_str()),
                Some(format!("@svc/{name}").as_str()),
                "expected name preserved in {name}/package.json",
            );
            assert_eq!(
                pkg.get("private").and_then(|v| v.as_bool()),
                Some(true),
                "expected private preserved in {name}/package.json",
            );
            let engines = pkg
                .get("engines")
                .and_then(|v| v.as_object())
                .unwrap_or_else(|| panic!("expected engines object in {name}/package.json"));
            assert_eq!(
                engines.get("node").and_then(|v| v.as_str()),
                Some(">=20"),
                "expected engines.node=>=20 in {name}/package.json",
            );

            let tsc_content = fs::read_to_string(svc.join("tsconfig.json"))
                .unwrap_or_else(|e| panic!("missing tsconfig.json for {name}: {e}"));
            let tsc: serde_json::Value = serde_json::from_str(&tsc_content)
                .unwrap_or_else(|e| panic!("invalid tsconfig.json for {name}: {e}"));
            let opts = tsc
                .get("compilerOptions")
                .and_then(|v| v.as_object())
                .unwrap_or_else(|| panic!("expected compilerOptions in {name}/tsconfig.json"));
            assert_eq!(
                opts.get("target").and_then(|v| v.as_str()),
                Some("ES2022"),
                "expected compilerOptions.target=ES2022 in {name}/tsconfig.json",
            );
            assert_eq!(
                opts.get("module").and_then(|v| v.as_str()),
                Some("ESNext"),
                "expected compilerOptions.module preserved in {name}/tsconfig.json",
            );
            assert_eq!(
                opts.get("strict").and_then(|v| v.as_bool()),
                Some(true),
                "expected compilerOptions.strict preserved in {name}/tsconfig.json",
            );
        }
    }

    fn setup_yaml_workflow_update(variant: Variant) -> (CleanupGuard, String) {
        let dir = Path::new(ROOT).join(format!("yaml_workflow_update_{}", variant.label()));
        reset_dir(&dir);
        let workflows = dir.join(".github").join("workflows");
        fs::create_dir_all(&workflows).expect("failed to create workflows dir");
        let ci = "name: CI\non: [push]\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v3\n      - run: cargo test\n";
        let deploy = "name: Deploy\non:\n  push:\n    branches: [main]\njobs:\n  ship:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v3\n      - run: ./deploy.sh\n";
        fs::write(workflows.join("ci.yml"), ci).expect("seed ci.yml");
        fs::write(workflows.join("deploy.yml"), deploy).expect("seed deploy.yml");
        (
            CleanupGuard::new().dir(dir.display().to_string()),
            dir.display().to_string(),
        )
    }

    fn assert_yaml_workflow_update(root: &str) {
        let workflows = Path::new(root).join(".github").join("workflows");
        for (name, must_contain) in [("ci.yml", "cargo test"), ("deploy.yml", "./deploy.sh")] {
            let content = fs::read_to_string(workflows.join(name))
                .unwrap_or_else(|e| panic!("missing {name}: {e}"));
            assert!(
                content.contains("actions/checkout@v4"),
                "expected {name} to be updated to actions/checkout@v4",
            );
            assert!(
                !content.contains("actions/checkout@v3"),
                "expected {name} to no longer reference actions/checkout@v3",
            );
            assert!(
                content.contains(must_contain),
                "expected {name} to preserve `{must_contain}` step",
            );
        }
    }

    fn setup_cargo_toml_rust_version_sync(variant: Variant) -> (CleanupGuard, String) {
        let dir = Path::new(ROOT).join(format!("cargo_toml_rust_version_sync_{}", variant.label()));
        reset_dir(&dir);

        fs::write(
            dir.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/alpha\", \"crates/beta\", \"crates/gamma\"]\nresolver = \"2\"\n",
        )
        .expect("seed root Cargo.toml");

        let crates_dir = dir.join("crates");
        for (name, content) in [
            (
                "alpha",
                "[package]\nname = \"alpha\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nserde = \"1\"\n",
            ),
            (
                "beta",
                "[package]\nname = \"beta\"\nversion = \"0.2.0\"\nedition = \"2021\"\nrust-version = \"1.80\"\n\n[dependencies]\nanyhow = \"1\"\n",
            ),
            (
                "gamma",
                "[package]\nname = \"gamma\"\nversion = \"0.3.0\"\nedition = \"2021\"\nrust-version = \"1.85\"\n\n[dependencies]\ntokio = { version = \"1\", features = [\"rt\"] }\n",
            ),
        ] {
            let crate_dir = crates_dir.join(name);
            fs::create_dir_all(&crate_dir).expect("failed to create crate dir");
            fs::write(crate_dir.join("Cargo.toml"), content)
                .unwrap_or_else(|e| panic!("seed {name}/Cargo.toml: {e}"));
        }

        (
            CleanupGuard::new().dir(dir.display().to_string()),
            dir.display().to_string(),
        )
    }

    fn assert_cargo_toml_rust_version_sync(root: &str) {
        let crates_dir = Path::new(root).join("crates");
        for (name, version, dep_key) in [
            ("alpha", "0.1.0", "serde"),
            ("beta", "0.2.0", "anyhow"),
            ("gamma", "0.3.0", "tokio"),
        ] {
            let path = crates_dir.join(name).join("Cargo.toml");
            let content = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("missing {name}/Cargo.toml: {e}"));
            assert_eq!(
                content.matches("rust-version = \"1.85\"").count(),
                1,
                "expected exactly one rust-version line in {name}/Cargo.toml, got: {content:?}"
            );
            let value: toml::Value = toml::from_str(&content)
                .unwrap_or_else(|e| panic!("invalid TOML in {name}/Cargo.toml: {e}\n{content}"));
            let package = value
                .get("package")
                .and_then(|v| v.as_table())
                .unwrap_or_else(|| panic!("missing [package] in {name}/Cargo.toml"));
            assert_eq!(
                package.get("name").and_then(|v| v.as_str()),
                Some(name),
                "expected package.name to be preserved in {name}/Cargo.toml"
            );
            assert_eq!(
                package.get("version").and_then(|v| v.as_str()),
                Some(version),
                "expected package.version to be preserved in {name}/Cargo.toml"
            );
            assert_eq!(
                package.get("edition").and_then(|v| v.as_str()),
                Some("2021"),
                "expected package.edition to be preserved in {name}/Cargo.toml"
            );
            assert_eq!(
                package.get("rust-version").and_then(|v| v.as_str()),
                Some("1.85"),
                "expected package.rust-version to be set in {name}/Cargo.toml"
            );
            let dependencies = value
                .get("dependencies")
                .and_then(|v| v.as_table())
                .unwrap_or_else(|| panic!("missing [dependencies] in {name}/Cargo.toml"));
            assert!(
                dependencies.contains_key(dep_key),
                "expected dependency `{dep_key}` to be preserved in {name}/Cargo.toml"
            );
        }
    }

    fn assert_completed_process_recovery(root: &str) {
        let path = Path::new(root).join("app.conf");
        let content = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("missing app.conf after recovery scenario: {e}"));
        assert!(
            content.lines().any(|line| line.trim() == "name=demo"),
            "expected app.conf to preserve name=demo, got: {content:?}"
        );
        assert!(
            content.lines().any(|line| line.trim() == "mode=enabled"),
            "expected app.conf to contain mode=enabled, got: {content:?}"
        );
    }

    fn assert_http_response_result_object(root: &str) {
        let path = Path::new(root).join("status.txt");
        let content = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("missing status.txt after http scenario: {e}"));
        assert!(
            content.lines().any(|line| line.trim() == "status=done"),
            "expected status.txt to contain status=done, got: {content:?}"
        );
        assert!(
            content.lines().any(|line| line.trim() == "owner=alice"),
            "expected status.txt to preserve owner=alice, got: {content:?}"
        );
        assert!(
            content.lines().any(|line| line.trim() == "reviewed=true"),
            "expected status.txt to contain reviewed=true, got: {content:?}"
        );
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

    fn completed_process_result_object_prompt(root: &str, rich_result_objects: bool) -> String {
        let preamble = format!(
            "Use a Python CodeAct script, not plain prose. In the directory {root:?}, run `./emit_lines.sh` with the `run(...)` helper using that directory as the workdir. The script prints exactly two lines: `alpha` and `beta`. Verify the command succeeded, split stdout into non-empty lines, and call FINAL exactly with `lines=alpha,beta`."
        );
        if rich_result_objects {
            format!(
                "{preamble} IMPORTANT: this is the RICH variant. Rich host result objects are enabled, so treat `await run(...)` as returning a `CompletedProcess` object. Use `proc.check_returncode()` to verify success, then read `proc.stdout` to build the final answer."
            )
        } else {
            format!(
                "{preamble} IMPORTANT: this is the SHIM baseline variant, not the rich-object variant. Treat `await run(...)` as the current normalized dict-like result with fields like `ok`, `exit_code`, `stdout`, `stderr`, and `sandboxed`. Do not assume object methods like `check_returncode()` are available."
            )
        }
    }

    fn completed_process_recovery_prompt(root: &str, rich_result_objects: bool) -> String {
        let preamble = format!(
            "Use a Python CodeAct script, not plain prose. In the directory {root:?}, there is an `app.conf` file and a validator script `./check_config.sh`. Run the validator first. It should fail because `app.conf` is missing one required line. Inspect the failure output to determine what line is missing, update `app.conf` to add that line while preserving the existing `name=demo` line, rerun the validator, verify success, then re-read `app.conf` and call FINAL exactly with `config=name=demo,mode=enabled`. You may use `read_text(...)` and `write_text(...)` for the file edit."
        );
        if rich_result_objects {
            format!(
                "{preamble} IMPORTANT: this is the RICH variant. Rich host result objects are enabled, so `await run(...)` returns a `CompletedProcess`. Use the first process object's `stderr` to learn what is missing. After fixing the file, rerun the validator and call `check.check_returncode()` on the successful second run before finalizing."
            )
        } else {
            format!(
                "{preamble} IMPORTANT: this is the SHIM baseline variant, not the rich-object variant. Treat `await run(...)` as the current normalized dict-like result with fields like `ok`, `exit_code`, `stdout`, `stderr`, and `sandboxed`. Use `.get(...)` access on that result and do not assume object methods like `check_returncode()` exist."
            )
        }
    }

    fn http_response_result_object_prompt(
        root: &str,
        api_base: &str,
        rich_result_objects: bool,
    ) -> String {
        let preamble = format!(
            "Use a Python CodeAct script, not plain prose. In the directory {root:?}, there is a `status.txt` file. First fetch the update plan from {api_base:?}/plan using `http_request('GET', url)`. The JSON response body includes exactly two keys: `required_status` and `required_marker`. Update `status.txt` so it keeps `owner=alice`, changes the status line to `status=<required_status>`, and ensures the exact `required_marker` line exists. Then POST the updated file content as JSON to {api_base:?}/validate using `http_request('POST', url, json={{'content': text}})`. If validation succeeds, re-read `status.txt` and call FINAL exactly with `state=status=done|reviewed=true`. You may use `read_text(...)` and `write_text(...)` for the file edit."
        );
        if rich_result_objects {
            format!(
                "{preamble} IMPORTANT: this is the RICH variant. Rich host result objects are enabled, so `http_request(...)` returns an `HttpResponse`. Use `plan.status` plus `plan.json()` for the first call, then `validation.status` plus `validation.json()` for the second call."
            )
        } else {
            format!(
                "{preamble} IMPORTANT: this is the SHIM baseline variant, not the rich-object variant. Treat `http_request(...)` as the current normalized dict-like result with fields like `status`, `json_body`, `text`, and `headers`. Use `.get('status')` and `.get('json_body')` style access, and do not assume methods like `.json()` exist."
            )
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

    fn monorepo_package_migration_prompt(root: &str, variant: Variant) -> String {
        let preamble = format!(
            "Use a Python CodeAct script, not plain prose. Under the monorepo root {root:?}, every package has a sub-directory at `packages/<name>/package.json`. For each package, ensure that the `scripts` object contains an entry `lint: eslint .` (add it if missing, do not overwrite an existing lint entry). Preserve every other existing script and all other top-level fields like `name` and `private`. Write each updated package.json back as pretty JSON. Then re-read every saved package.json to build a summary. For each package, sort its script keys alphabetically and join them with `,`. Join all packages sorted by package name with `;`. Call FINAL exactly with `summary=<summary>` — for example `admin:build,lint;app:lint,test;docs:lint,preview`."
        );
        match variant {
            Variant::Raw => format!(
                "{preamble} IMPORTANT: this is the RAW variant, so do not use helper shims like read_json, write_json, read_text, write_text, list_entries, or find_files. Use canonical host tools directly from Python. The canonical read_file tool returns a dict whose `content` field contains numbered text in a format like `     1│ {{...}}`; strip everything through the `│` separator before JSON parsing. You can discover package directories with the canonical `list_dir` tool; its output is a dict with an `entries` list where directories end with `/`."
            ),
            Variant::Shim => format!(
                "{preamble} IMPORTANT: this is the SHIM variant, so prefer `await read_json(path)` and `await write_json(path, value)` for each package.json, and `await list_entries(path)` to discover package directories. All three helpers are async, so always use `await`. They are thin facades over the canonical host tools and keep the same approval/policy behavior."
            ),
        }
    }

    fn tsconfig_nested_paths_prompt(path: &str, variant: Variant) -> String {
        let preamble = format!(
            "Use a Python CodeAct script, not plain prose. Edit the repo file {path:?} as if you were updating a realistic nested tsconfig.json. Inside `compilerOptions`, set `strict` to true and add `noUncheckedIndexedAccess: true`. Inside `compilerOptions.paths`, add a new mapping `@ui/*: [\"src/ui/*\"]` while preserving the existing `@app/*` and `@lib/*` mappings. Preserve every other existing field including `target`, `module`, top-level `include`, and top-level `exclude`. Write the updated file back as pretty JSON, then re-read the saved file. Extract the keys of `compilerOptions.paths` sorted alphabetically and call FINAL exactly with `paths=<keys joined by commas>` — for example `paths=@app/*,@lib/*,@ui/*`."
        );
        match variant {
            Variant::Raw => format!(
                "{preamble} IMPORTANT: this is the RAW variant, so do not use helper shims like read_json, write_json, read_text, or write_text. Use canonical host tools directly from Python. The canonical read_file tool returns a dict whose `content` field contains numbered text in a format like `     1│ {{...}}`; strip everything through the `│` separator before JSON parsing."
            ),
            Variant::Shim => format!(
                "{preamble} IMPORTANT: this is the SHIM variant, so prefer `await read_json(path)` and `await write_json(path, value)`. Both are async, so always use `await`."
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

    fn js_codemod_use_strict_prompt(root: &str, variant: Variant) -> String {
        let preamble = format!(
            "Use a Python CodeAct script, not plain prose. Under the directory {root:?}, find every `.js` file recursively under `src/`. For each file, prepend the literal line `'use strict';\\n` to its content. Preserve every other byte of the existing content exactly (do not add extra whitespace, blank lines, or remove the trailing newline). After updating every file, call FINAL exactly with `done=ok`."
        );
        match variant {
            Variant::Raw => format!(
                "{preamble} IMPORTANT: this is the RAW variant, so do not use helper shims like find_files, read_text, or write_text. Use canonical host tools directly from Python. The canonical glob tool returns a dict with a `files` list of relative paths matching the pattern. The canonical read_file tool returns a dict whose `content` field contains numbered text in a format like `     1│ <text>`; strip everything through the `│` separator before checking the first line. Always pass the full absolute path returned by glob to read_file (do not use just the basename)."
            ),
            Variant::Shim => format!(
                "{preamble} IMPORTANT: this is the SHIM variant, so prefer `await find_files('**/*.js', path=...)`, `await read_text(path)`, and `await write_text(path, content)`. The text shims return and accept raw file content directly with no line-number framing. All shims are async, so always use `await`."
            ),
        }
    }

    fn mixed_config_sync_prompt(root: &str, variant: Variant) -> String {
        let preamble = format!(
            "Use a Python CodeAct script, not plain prose. Under the directory {root:?}, every service has both a `services/<name>/package.json` and a `services/<name>/tsconfig.json`. For each service, ensure the package.json has an object field `engines` with an entry `node: '>=20'` (add the engines object if missing; preserve every other top-level field including `name`, `private`, `version`). For each service, also ensure the tsconfig.json's `compilerOptions` object contains an entry `target: 'ES2022'` (add it if missing; preserve every other field in compilerOptions like `module` and `strict`). Write each updated file back as pretty JSON. Then re-read every saved file to verify, and call FINAL exactly with `done=<comma-joined sorted service names>` — for example `done=api,web,worker`. Only include services where BOTH files now have the required entries."
        );
        match variant {
            Variant::Raw => format!(
                "{preamble} IMPORTANT: this is the RAW variant, so do not use helper shims like read_json, write_json, list_entries, or read_text. Use canonical host tools directly from Python. The canonical read_file tool returns a dict whose `content` field contains numbered text in a format like `     1│ {{...}}`; strip everything through the `│` separator before JSON parsing. Discover service directories with the canonical `list_dir` tool; its output is a dict with an `entries` list where directories end with `/`."
            ),
            Variant::Shim => format!(
                "{preamble} IMPORTANT: this is the SHIM variant, so prefer `await list_entries(path)` to discover service directories, and `await read_json(path)` / `await write_json(path, value)` for each JSON file. All shims are async, so always use `await`."
            ),
        }
    }

    fn yaml_workflow_update_prompt(root: &str, variant: Variant) -> String {
        let preamble = format!(
            "Use a Python CodeAct script, not plain prose. Under the directory {root:?}, find every `.yml` file recursively under `.github/workflows/`. For each file, replace every literal occurrence of the string `actions/checkout@v3` with `actions/checkout@v4`. Preserve all other content exactly. Write each updated file back, then re-read every workflow to verify the substitution. Build a sorted list of the file basenames you actually modified and call FINAL exactly with `updated=<comma-joined sorted basenames>` — for example `updated=ci.yml,deploy.yml`."
        );
        match variant {
            Variant::Raw => format!(
                "{preamble} IMPORTANT: this is the RAW variant, so do not use helper shims like find_files, read_text, or write_text. Use canonical host tools directly from Python. The canonical glob tool returns a dict with a `files` list of relative paths. The canonical read_file tool returns a dict whose `content` field contains numbered text in a format like `     1│ <text>`; strip everything through the `│` separator before doing the substitution. Do not import os; derive each file's basename by splitting its path on '/'."
            ),
            Variant::Shim => format!(
                "{preamble} IMPORTANT: this is the SHIM variant, so prefer `await find_files('**/*.yml', path=...)`, `await read_text(path)`, and `await write_text(path, content)`. The text shims return and accept raw file content directly with no line-number framing. All shims are async, so always use `await`. Derive each file's basename by splitting its returned path on '/'."
            ),
        }
    }

    fn cargo_toml_rust_version_sync_prompt(root: &str, variant: Variant) -> String {
        let crates_root = Path::new(root).join("crates");
        let preamble = format!(
            "Use a Python CodeAct script, not plain prose. Under the directory {root:?}, every member crate lives at `crates/<name>/Cargo.toml`. For each member Cargo.toml under {crates_root:?}, ensure the `[package]` section contains exactly one line `rust-version = \"1.85\"`. If the file already has a `rust-version = ...` line in `[package]`, replace its value with `\"1.85\"`. If it is missing, insert `rust-version = \"1.85\"` immediately after the `edition = \"2021\"` line. Preserve every other line exactly, including dependencies and blank lines. After writing, re-read every member Cargo.toml and verify it now contains exactly one `rust-version = \"1.85\"` line. Then call FINAL exactly with `done=alpha,beta,gamma`."
        );
        match variant {
            Variant::Raw => format!(
                "{preamble} IMPORTANT: this is the RAW variant, so do not use helper shims like find_files, read_text, or write_text. Use canonical host tools directly from Python. Use the canonical `glob` tool to discover member Cargo.toml files. It returns paths relative to the search root, so prefix each returned path with the absolute crates root {crates_root:?} before calling `read_file` or `write_file`. The canonical read_file tool returns a dict whose `content` field contains numbered text like `     1│ <text>`; strip everything through the `│` separator before editing. Avoid importing external parsers; do this as line-oriented text editing."
            ),
            Variant::Shim => format!(
                "{preamble} IMPORTANT: this is the SHIM variant, so prefer `await find_files('**/Cargo.toml', path={crates_root:?})`, `await read_text(path)`, and `await write_text(path, content)`. `find_files` returns paths relative to the search root, so prefix each returned path with the absolute crates root before reading or writing. The text shims return and accept raw text directly with no line-number framing. All shims are async, so always use `await`."
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

    #[tokio::test]
    #[ignore]
    async fn codeact_monorepo_package_migration_raw_vs_shim() {
        let _guard = engine_v2_live_lock().lock().await;
        if !should_run_fixture_pair("monorepo_package_migration") {
            return;
        }

        let (_cleanup, root) = setup_monorepo_package_migration();
        let raw = run_variant(
            "monorepo_package_migration",
            Variant::Raw,
            monorepo_package_migration_prompt(&root, Variant::Raw),
        )
        .await;
        assert_exact_suffix(
            &raw.response,
            "summary=",
            "admin:build,lint;app:lint,test;docs:lint,preview",
        );
        assert_monorepo_package_migration(&root);

        let (_cleanup, root) = setup_monorepo_package_migration();
        let shim = run_variant(
            "monorepo_package_migration",
            Variant::Shim,
            monorepo_package_migration_prompt(&root, Variant::Shim),
        )
        .await;
        assert_exact_suffix(
            &shim.response,
            "summary=",
            "admin:build,lint;app:lint,test;docs:lint,preview",
        );
        assert_monorepo_package_migration(&root);

        print_pair_report(&raw, &shim);
    }

    #[tokio::test]
    #[ignore]
    async fn codeact_tsconfig_nested_paths_raw_vs_shim() {
        let _guard = engine_v2_live_lock().lock().await;
        if !should_run_fixture_pair("tsconfig_nested_paths") {
            return;
        }

        let (_cleanup, path) = setup_tsconfig_nested_paths();
        let raw = run_variant(
            "tsconfig_nested_paths",
            Variant::Raw,
            tsconfig_nested_paths_prompt(&path, Variant::Raw),
        )
        .await;
        assert_exact_suffix(&raw.response, "paths=", "@app/*,@lib/*,@ui/*");
        assert_tsconfig_nested_paths(&path);

        let (_cleanup, path) = setup_tsconfig_nested_paths();
        let shim = run_variant(
            "tsconfig_nested_paths",
            Variant::Shim,
            tsconfig_nested_paths_prompt(&path, Variant::Shim),
        )
        .await;
        assert_exact_suffix(&shim.response, "paths=", "@app/*,@lib/*,@ui/*");
        assert_tsconfig_nested_paths(&path);

        print_pair_report(&raw, &shim);
    }

    #[tokio::test]
    #[ignore]
    async fn codeact_js_codemod_use_strict_raw_vs_shim() {
        let _guard = engine_v2_live_lock().lock().await;
        if !should_run_fixture_pair("js_codemod_use_strict") {
            return;
        }

        let (_cleanup_raw, raw_root) = setup_js_codemod_use_strict(Variant::Raw);
        let raw = run_variant_tolerant(
            "js_codemod_use_strict",
            Variant::Raw,
            js_codemod_use_strict_prompt(&raw_root, Variant::Raw),
        )
        .await;

        let (_cleanup_shim, shim_root) = setup_js_codemod_use_strict(Variant::Shim);
        let shim = run_variant_tolerant(
            "js_codemod_use_strict",
            Variant::Shim,
            js_codemod_use_strict_prompt(&shim_root, Variant::Shim),
        )
        .await;

        print_pair_report(&raw, &shim);
        assert_exact_suffix(&raw.response, "done=", "ok");
        assert_js_codemod_use_strict(&raw_root);
        assert_exact_suffix(&shim.response, "done=", "ok");
        assert_js_codemod_use_strict(&shim_root);
    }

    #[tokio::test]
    #[ignore]
    async fn codeact_mixed_config_sync_raw_vs_shim() {
        let _guard = engine_v2_live_lock().lock().await;
        if !should_run_fixture_pair("mixed_config_sync") {
            return;
        }

        let (_cleanup_raw, raw_root) = setup_mixed_config_sync(Variant::Raw);
        let raw = run_variant_tolerant(
            "mixed_config_sync",
            Variant::Raw,
            mixed_config_sync_prompt(&raw_root, Variant::Raw),
        )
        .await;

        let (_cleanup_shim, shim_root) = setup_mixed_config_sync(Variant::Shim);
        let shim = run_variant_tolerant(
            "mixed_config_sync",
            Variant::Shim,
            mixed_config_sync_prompt(&shim_root, Variant::Shim),
        )
        .await;

        print_pair_report(&raw, &shim);
        assert_exact_suffix(&raw.response, "done=", "api,web,worker");
        assert_mixed_config_sync(&raw_root);
        assert_exact_suffix(&shim.response, "done=", "api,web,worker");
        assert_mixed_config_sync(&shim_root);
    }

    #[tokio::test]
    #[ignore]
    async fn codeact_yaml_workflow_update_raw_vs_shim() {
        let _guard = engine_v2_live_lock().lock().await;
        if !should_run_fixture_pair("yaml_workflow_update") {
            return;
        }

        let (_cleanup_raw, raw_root) = setup_yaml_workflow_update(Variant::Raw);
        let raw = run_variant_tolerant(
            "yaml_workflow_update",
            Variant::Raw,
            yaml_workflow_update_prompt(&raw_root, Variant::Raw),
        )
        .await;

        let (_cleanup_shim, shim_root) = setup_yaml_workflow_update(Variant::Shim);
        let shim = run_variant_tolerant(
            "yaml_workflow_update",
            Variant::Shim,
            yaml_workflow_update_prompt(&shim_root, Variant::Shim),
        )
        .await;

        print_pair_report(&raw, &shim);
        // Raw variant commonly thrashes past its token budget on this scenario
        // and finalizes with an empty `updated=`. That's exactly the failure mode
        // the shim is meant to fix, so we assert the prefix was reached but tolerate
        // either success or thrash-empty — see shim assertion below for the strict check.
        assert!(
            raw.response.contains("updated="),
            "raw response should finalize with an 'updated=' prefix, got: {}",
            raw.response
        );
        assert_exact_suffix(&shim.response, "updated=", "ci.yml,deploy.yml");
        assert_yaml_workflow_update(&shim_root);
    }

    #[tokio::test]
    #[ignore]
    async fn codeact_cargo_toml_rust_version_sync_raw_vs_shim() {
        let _guard = engine_v2_live_lock().lock().await;
        if !should_run_fixture_pair("cargo_toml_rust_version_sync") {
            return;
        }

        let (_cleanup_raw, raw_root) = setup_cargo_toml_rust_version_sync(Variant::Raw);
        let raw = run_variant_tolerant(
            "cargo_toml_rust_version_sync",
            Variant::Raw,
            cargo_toml_rust_version_sync_prompt(&raw_root, Variant::Raw),
        )
        .await;

        let (_cleanup_shim, shim_root) = setup_cargo_toml_rust_version_sync(Variant::Shim);
        let shim = run_variant_tolerant(
            "cargo_toml_rust_version_sync",
            Variant::Shim,
            cargo_toml_rust_version_sync_prompt(&shim_root, Variant::Shim),
        )
        .await;

        print_pair_report(&raw, &shim);
        assert_exact_suffix(&raw.response, "done=", "alpha,beta,gamma");
        assert_cargo_toml_rust_version_sync(&raw_root);
        assert_exact_suffix(&shim.response, "done=", "alpha,beta,gamma");
        assert_cargo_toml_rust_version_sync(&shim_root);
    }

    #[tokio::test]
    #[ignore]
    async fn codeact_completed_process_shim_vs_rich_objects() {
        let _guard = engine_v2_live_lock().lock().await;
        if !should_run_fixture_names("completed_process_result_object", &["shim", "rich"]) {
            return;
        }

        let (_cleanup_shim, shim_root) = setup_completed_process_result_object("shim");
        let shim = run_named_variant(
            "completed_process_result_object",
            "shim",
            completed_process_result_object_prompt(&shim_root, false),
            false,
        )
        .await;

        let (_cleanup_rich, rich_root) = setup_completed_process_result_object("rich");
        let rich = run_named_variant(
            "completed_process_result_object",
            "rich",
            completed_process_result_object_prompt(&rich_root, true),
            true,
        )
        .await;

        print_pair_report(&shim, &rich);
        assert_exact_suffix(&shim.response, "lines=", "alpha,beta");
        assert_exact_suffix(&rich.response, "lines=", "alpha,beta");
        assert!(
            shim.tool_calls_started
                .iter()
                .any(|name| name.starts_with("shell")),
            "shim baseline should call shell, got {:?}",
            shim.tool_calls_started
        );
        assert!(
            rich.tool_calls_started
                .iter()
                .any(|name| name.starts_with("shell")),
            "rich variant should still call canonical shell, got {:?}",
            rich.tool_calls_started
        );

        let shim_code = recorded_text_response("completed_process_result_object", "shim");
        assert!(
            shim_code.contains("result.get(\"stdout\"")
                || shim_code.contains("result.get('stdout'"),
            "shim baseline should use dict-like stdout access, got: {}",
            shim_code
        );
        assert!(
            !shim_code.contains("check_returncode"),
            "shim baseline should not use rich-object methods, got: {}",
            shim_code
        );

        let rich_code = recorded_text_response("completed_process_result_object", "rich");
        assert!(
            rich_code.contains("check_returncode()"),
            "rich variant should exercise CompletedProcess.check_returncode(), got: {}",
            rich_code
        );
        assert!(
            rich_code.contains("proc.stdout"),
            "rich variant should use attribute access on CompletedProcess, got: {}",
            rich_code
        );
    }

    #[tokio::test]
    #[ignore]
    async fn codeact_completed_process_recovery_shim_vs_rich_objects() {
        let _guard = engine_v2_live_lock().lock().await;
        if !should_run_fixture_names("completed_process_recovery", &["shim", "rich"]) {
            return;
        }

        let (_cleanup_shim, shim_root) = setup_completed_process_recovery("shim");
        let shim = run_named_variant(
            "completed_process_recovery",
            "shim",
            completed_process_recovery_prompt(&shim_root, false),
            false,
        )
        .await;

        let (_cleanup_rich, rich_root) = setup_completed_process_recovery("rich");
        let rich = run_named_variant(
            "completed_process_recovery",
            "rich",
            completed_process_recovery_prompt(&rich_root, true),
            true,
        )
        .await;

        print_pair_report(&shim, &rich);
        assert_exact_suffix(&shim.response, "config=", "name=demo,mode=enabled");
        assert_exact_suffix(&rich.response, "config=", "name=demo,mode=enabled");
        assert_completed_process_recovery(&shim_root);
        assert_completed_process_recovery(&rich_root);
        assert!(
            shim.tool_calls_started
                .iter()
                .filter(|name| name.starts_with("shell"))
                .count()
                >= 2,
            "shim recovery variant should run the validator at least twice, got {:?}",
            shim.tool_calls_started
        );
        assert!(
            rich.tool_calls_started
                .iter()
                .filter(|name| name.starts_with("shell"))
                .count()
                >= 2,
            "rich recovery variant should run the validator at least twice, got {:?}",
            rich.tool_calls_started
        );

        let shim_code = recorded_text_response("completed_process_recovery", "shim");
        assert!(
            shim_code.contains("get(\"stderr\"") || shim_code.contains("get('stderr'"),
            "shim recovery variant should inspect stderr through dict-like access, got: {}",
            shim_code
        );
        assert!(
            !shim_code.contains("check_returncode"),
            "shim recovery variant should not use rich-object methods, got: {}",
            shim_code
        );

        let rich_code = recorded_text_response("completed_process_recovery", "rich");
        assert!(
            rich_code.contains(".stderr"),
            "rich recovery variant should inspect CompletedProcess.stderr, got: {}",
            rich_code
        );
        assert!(
            rich_code.contains("check_returncode()"),
            "rich recovery variant should use CompletedProcess.check_returncode(), got: {}",
            rich_code
        );
    }

    #[tokio::test]
    #[ignore]
    async fn codeact_http_response_shim_vs_rich_objects() {
        let _guard = engine_v2_live_lock().lock().await;
        if !should_run_fixture_names("http_response_result_object", &["shim", "rich"]) {
            return;
        }

        let (_cleanup_shim, shim_root, shim_api) = setup_http_response_result_object("shim").await;
        let shim = run_named_variant(
            "http_response_result_object",
            "shim",
            http_response_result_object_prompt(&shim_root, &shim_api, false),
            false,
        )
        .await;

        let (_cleanup_rich, rich_root, rich_api) = setup_http_response_result_object("rich").await;
        let rich = run_named_variant(
            "http_response_result_object",
            "rich",
            http_response_result_object_prompt(&rich_root, &rich_api, true),
            true,
        )
        .await;

        print_pair_report(&shim, &rich);
        assert_exact_suffix(&shim.response, "state=", "status=done|reviewed=true");
        assert_exact_suffix(&rich.response, "state=", "status=done|reviewed=true");
        assert_http_response_result_object(&shim_root);
        assert_http_response_result_object(&rich_root);
        assert!(
            shim.tool_calls_started
                .iter()
                .filter(|name| name.starts_with("http"))
                .count()
                >= 2,
            "shim http variant should make at least two canonical http calls, got {:?}",
            shim.tool_calls_started
        );
        assert!(
            rich.tool_calls_started
                .iter()
                .filter(|name| name.starts_with("http"))
                .count()
                >= 2,
            "rich http variant should make at least two canonical http calls, got {:?}",
            rich.tool_calls_started
        );

        let shim_code = recorded_text_response("http_response_result_object", "shim");
        assert!(
            shim_code.contains("get(\"json_body\"") || shim_code.contains("get('json_body'"),
            "shim http variant should use dict-like json_body access, got: {}",
            shim_code
        );
        assert!(
            !shim_code.contains("plan.json()") && !shim_code.contains("validation.json()"),
            "shim http variant should not use HttpResponse.json(), got: {}",
            shim_code
        );

        let rich_code = recorded_text_response("http_response_result_object", "rich");
        assert!(
            rich_code.contains("plan.status") || rich_code.contains("validation.status"),
            "rich http variant should use HttpResponse.status, got: {}",
            rich_code
        );
        assert!(
            rich_code.contains("plan.json()") || rich_code.contains("validation.json()"),
            "rich http variant should use HttpResponse.json(), got: {}",
            rich_code
        );
    }
}
