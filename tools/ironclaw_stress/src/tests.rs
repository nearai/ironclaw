use super::*;

#[test]
fn redacts_postgres_uri_credentials_but_keeps_host() {
    let redacted = redact_postgres_url("postgres://user:secret@localhost:5432/app");

    assert_eq!(redacted, "postgres://<redacted>@localhost:5432/app");
    assert!(!redacted.contains("secret"));
}

#[test]
fn redacts_postgresql_uri_password_query_parameter() {
    let redacted =
        redact_postgres_url("postgresql://localhost/app?sslmode=require&password=secret");

    assert_eq!(
        redacted,
        "postgresql://localhost/app?sslmode=require&password=<redacted>"
    );
    assert!(!redacted.contains("secret"));
}

#[test]
fn redacts_key_value_postgres_password() {
    let redacted = redact_postgres_url("host=localhost user=postgres password=secret dbname=app");

    assert_eq!(
        redacted,
        "host=localhost user=postgres password=<redacted> dbname=app"
    );
    assert!(!redacted.contains("secret"));
}

#[test]
fn redacts_libsql_absolute_path() {
    let redacted = redact_libsql_path(Path::new("/tmp/ironclaw-stress-secret.db"));

    assert_eq!(redacted, "libsql://<redacted-local-path>");
    assert!(!redacted.contains("/tmp"));
}

#[test]
fn synthetic_ids_are_generated_once_for_requested_cardinality() {
    let args = test_args();
    let ids = SyntheticIds::new(&args).expect("synthetic ids build");

    assert_eq!(ids.tenant_count(), args.tenants);
    assert_eq!(ids.user_count(), args.users);
}

#[test]
fn active_thread_count_reuses_hot_thread_but_keeps_actor_fanout() {
    let mut args = test_args();
    args.users = 4;
    args.active_thread_count = 1;
    let ids = SyntheticIds::new(&args).expect("synthetic ids build");

    let first = ids.user_turn_context(&args, 0, 0).expect("first context");
    let second = ids.user_turn_context(&args, 0, 1).expect("second context");

    assert_ne!(first.user_id.as_str(), second.user_id.as_str());
    assert_eq!(first.thread_id.as_str(), second.thread_id.as_str());
    assert_eq!(
        first.thread_owner_user_id.as_str(),
        second.thread_owner_user_id.as_str()
    );
}

#[test]
fn fixed_operation_mode_spreads_concurrent_workers_across_threads() {
    let mut args = test_args();
    args.users = 8;
    args.concurrency = 8;
    args.operations = 200;
    args.active_thread_count = 0;
    let ids = SyntheticIds::new(&args).expect("synthetic ids build");

    let thread_ids = (0..args.concurrency)
        .map(|worker_index| {
            ids.user_turn_context(&args, worker_index, 0)
                .expect("context")
                .thread_id
        })
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(thread_ids.len(), args.concurrency);
}

#[test]
fn chat_turn_rejects_multi_process_runs() {
    let mut args = test_args();
    args.scenario = Scenario::ChatTurn;
    args.processes = 2;

    let error = validate_args(&args).expect_err("chat-turn is single-process only");

    assert!(error.contains("--scenario chat-turn requires --processes 1"));
}

#[test]
fn mixed_user_session_rejects_multi_process_runs() {
    let mut args = test_args();
    args.scenario = Scenario::MixedUserSession;
    args.processes = 2;

    let error = validate_args(&args).expect_err("mixed sessions are single-process only");

    assert!(error.contains("--scenario mixed-user-session requires --processes 1"));
}

#[test]
fn large_context_preset_populates_workload_defaults() {
    let args = parse_test_args([
        "ironclaw_stress",
        "--backend",
        "libsql",
        "--preset",
        "large-context",
    ]);

    assert_eq!(args.preset, Some(StressPreset::LargeContext));
    assert_eq!(args.scenario, Scenario::MixedUserSession);
    assert_eq!(args.concurrency, 4);
    assert_eq!(args.operations, 50);
    assert_eq!(args.users, 100);
    assert_eq!(args.prefill_threads, 100);
    assert_eq!(args.prefill_turns_per_thread, 50);
    assert_eq!(args.context_max_messages, 100);
}

#[test]
fn preset_respects_explicit_overrides() {
    let args = parse_test_args([
        "ironclaw_stress",
        "--backend",
        "libsql",
        "--preset",
        "large-context",
        "--users",
        "7",
        "--context-max-messages",
        "11",
    ]);

    assert_eq!(args.users, 7);
    assert_eq!(args.prefill_threads, 7);
    assert_eq!(args.context_max_messages, 11);
}

#[test]
fn bottleneck_finder_suite_includes_core_pressure_cases() {
    let cases = suite::build_cases(StressSuite::BottleneckFinder);
    let labels = cases
        .iter()
        .map(|case| case.label)
        .collect::<std::collections::BTreeSet<_>>();

    assert!(labels.contains("resource-contention"));
    assert!(labels.contains("chat-baseline"));
    assert!(labels.contains("hot-thread"));
    assert!(labels.contains("large-context"));
    assert!(labels.contains("tool-heavy"));
    assert!(labels.contains("tool-wait"));
    assert!(labels.contains("tool-failure"));
    assert!(labels.contains("model-tail"));
    assert!(labels.contains("cpu-burn"));
    assert!(labels.contains("memory-churn"));
}

#[test]
fn postgres_pool_pressure_suite_includes_remote_pool_cases() {
    let cases = suite::build_cases(StressSuite::PostgresPoolPressure);
    let labels = cases
        .iter()
        .map(|case| case.label)
        .collect::<std::collections::BTreeSet<_>>();

    assert!(labels.contains("postgres-chat-pool"));
    assert!(labels.contains("postgres-hot-thread-pool"));
    assert!(labels.contains("postgres-context-pool"));
    assert!(labels.contains("postgres-tool-pool"));
}

#[test]
fn log_prefix_includes_suite_case_label() {
    let mut args = test_args();
    args.suite_case_label = Some("large-context".to_string());

    assert_eq!(log_prefix(&args), "[ironclaw-stress case=large-context]");

    args.child_index = Some(2);
    assert_eq!(
        log_prefix(&args),
        "[ironclaw-stress child=2 case=large-context]"
    );
}

#[test]
fn soak_user_session_preset_uses_duration_mode() {
    let args = parse_test_args([
        "ironclaw_stress",
        "--backend",
        "libsql",
        "--preset",
        "soak-user-session",
    ]);

    assert_eq!(args.preset, Some(StressPreset::SoakUserSession));
    assert_eq!(args.scenario, Scenario::MixedUserSession);
    assert_eq!(args.duration_seconds, 900);
    assert_eq!(args.warmup_seconds, 60);
    assert_eq!(args.trace_interval_seconds, 30);
    assert_eq!(args.prefill_turns_per_thread, 20);
}

#[test]
fn suite_rejects_multi_process_runs() {
    let mut args = test_args();
    args.suite = Some(StressSuite::BottleneckFinder);
    args.processes = 2;

    let error = validate_args(&args).expect_err("suite should reject multi-process runs");

    assert!(error.contains("--suite requires --processes 1"));
}

#[test]
fn postgres_pool_pressure_suite_requires_postgres_backend() {
    let mut args = test_args();
    args.suite = Some(StressSuite::PostgresPoolPressure);
    args.backend = Backend::Libsql;

    let error = validate_args(&args).expect_err("postgres suite should reject libsql backend");

    assert!(error.contains("--suite postgres-pool-pressure requires --backend postgres"));
}

#[test]
fn suite_rejects_preset_runs() {
    let mut args = test_args();
    args.suite = Some(StressSuite::BottleneckFinder);
    args.preset = Some(StressPreset::LargeContext);

    let error = validate_args(&args).expect_err("suite should reject preset runs");

    assert!(error.contains("--suite cannot be combined with --preset"));
}

#[test]
fn context_growth_rejects_multi_process_runs() {
    let mut args = test_args();
    args.scenario = Scenario::ContextGrowth;
    args.processes = 2;

    let error = validate_args(&args).expect_err("context-growth is single-process only");

    assert!(error.contains("--scenario context-growth requires --processes 1"));
}

#[test]
fn tool_session_rejects_multi_process_runs() {
    let mut args = test_args();
    args.scenario = Scenario::ToolSession;
    args.processes = 2;

    let error = validate_args(&args).expect_err("tool-session is single-process only");

    assert!(error.contains("--scenario tool-session requires --processes 1"));
}

#[test]
fn context_growth_rejects_zero_turns_per_operation() {
    let mut args = test_args();
    args.context_growth_turns_per_operation = 0;

    let error = validate_args(&args).expect_err("zero context growth turns is invalid");

    assert!(error.contains("--context-growth-turns-per-operation"));
}

#[test]
fn tool_session_rejects_zero_tool_calls() {
    let mut args = test_args();
    args.tool_calls_per_turn = 0;

    let error = validate_args(&args).expect_err("zero tool calls are invalid");

    assert!(error.contains("--tool-calls-per-turn"));
}

#[test]
fn tool_session_rejects_oversized_preview_output() {
    let mut args = test_args();
    args.tool_output_bytes = 16 * 1024 + 1;

    let error = validate_args(&args).expect_err("oversized tool preview is invalid");

    assert!(error.contains("--tool-output-bytes"));
}

#[test]
fn prefill_requires_both_thread_and_turn_counts() {
    let mut args = test_args();
    args.scenario = Scenario::ChatTurn;
    args.prefill_threads = 2;

    let error = validate_args(&args).expect_err("partial prefill settings are invalid");

    assert!(error.contains("--prefill-threads and --prefill-turns-per-thread"));
}

#[test]
fn prefill_rejects_non_user_turn_scenarios() {
    let mut args = test_args();
    args.scenario = Scenario::ReserveRelease;
    args.prefill_threads = 2;
    args.prefill_turns_per_thread = 3;

    let error = validate_args(&args).expect_err("resource scenario prefill is invalid");

    assert!(error.contains("--prefill-threads requires a user-turn scenario"));
}

#[test]
fn prefill_rejects_more_threads_than_users() {
    let mut args = test_args();
    args.scenario = Scenario::ChatTurn;
    args.users = 2;
    args.prefill_threads = 3;
    args.prefill_turns_per_thread = 1;

    let error = validate_args(&args).expect_err("prefill thread count exceeds users");

    assert!(error.contains("--prefill-threads must be less than or equal to --users"));
}

#[test]
fn prefill_rejects_sweep_user_values_below_prefill_threads() {
    let mut args = test_args();
    args.scenario = Scenario::ChatTurn;
    args.users = 4;
    args.prefill_threads = 3;
    args.prefill_turns_per_thread = 1;
    args.sweep_users = vec![2, 4];

    let error = validate_args(&args).expect_err("sweep users below prefill threads");

    assert!(error.contains("every --sweep-users value"));
}

#[test]
fn warmup_requires_duration_mode() {
    let mut args = test_args();
    args.duration_seconds = 0;
    args.warmup_seconds = 1;

    let error = validate_args(&args).expect_err("warmup without duration is invalid");

    assert!(error.contains("--warmup-seconds requires --duration-seconds"));
}

#[test]
fn duration_mode_has_no_fixed_progress_total() {
    let mut args = test_args();
    args.duration_seconds = 10;

    assert!(matches!(
        args.operation_target(),
        OperationTarget::Duration { .. }
    ));
    assert_eq!(args.operation_target().progress_total(), None);
}

#[test]
fn sweep_concurrency_rejects_zero_values() {
    let mut args = test_args();
    args.sweep_concurrency = vec![1, 0, 2];

    let error = validate_args(&args).expect_err("zero sweep concurrency is invalid");

    assert!(error.contains("--sweep-concurrency values must be greater than 0"));
}

#[test]
fn active_thread_count_rejects_values_above_users() {
    let mut args = test_args();
    args.active_thread_count = args.users + 1;

    let error = validate_args(&args).expect_err("active thread count exceeds users");

    assert!(error.contains("--active-thread-count"));
}

#[test]
fn sweep_active_thread_count_rejects_values_above_sweep_users() {
    let mut args = test_args();
    args.sweep_users = vec![2, 4];
    args.sweep_active_thread_count = vec![0, 3];

    let error = validate_args(&args).expect_err("active thread sweep exceeds users");

    assert!(error.contains("--sweep-active-thread-count"));
}

#[test]
fn sweep_payload_axes_enable_sweep() {
    let mut args = test_args();
    args.sweep_tool_output_bytes = vec![0, 1024];

    assert!(sweep::is_enabled(&args));
}

#[test]
fn sweep_context_axes_reject_zero_values() {
    let mut args = test_args();
    args.sweep_context_max_messages = vec![0];

    let error = validate_args(&args).expect_err("zero sweep context max is invalid");

    assert!(error.contains("--sweep-context-max-messages"));
}

#[test]
fn sweep_tool_call_axes_reject_zero_values() {
    let mut args = test_args();
    args.sweep_tool_calls_per_turn = vec![0];

    let error = validate_args(&args).expect_err("zero sweep tool calls are invalid");

    assert!(error.contains("--sweep-tool-calls-per-turn"));
}

#[test]
fn sweep_tool_output_axes_reject_oversized_values() {
    let mut args = test_args();
    args.sweep_tool_output_bytes = vec![16 * 1024 + 1];

    let error = validate_args(&args).expect_err("oversized sweep tool output is invalid");

    assert!(error.contains("--sweep-tool-output-bytes"));
}

#[test]
fn ramp_builds_bounded_geometric_values() {
    assert_eq!(ramp::build_values(3, 20, 2), vec![3, 6, 12, 20]);
    assert_eq!(ramp::build_values(4, 4, 2), vec![4]);
}

#[test]
fn ramp_rejects_multiple_axes() {
    let mut args = test_args();
    args.ramp_concurrency = Some(8);
    args.ramp_users = Some(100);

    let error = validate_args(&args).expect_err("multiple ramp axes are invalid");

    assert!(error.contains("use only one of --ramp-concurrency or --ramp-users"));
}

#[test]
fn ramp_rejects_sweep_flags() {
    let mut args = test_args();
    args.ramp_concurrency = Some(8);
    args.sweep_users = vec![10, 20];

    let error = validate_args(&args).expect_err("ramp and sweep cannot combine");

    assert!(error.contains("ramp mode cannot be combined with sweep flags"));
}

#[test]
fn ramp_rejects_factor_one() {
    let mut args = test_args();
    args.ramp_concurrency = Some(8);
    args.ramp_factor = 1;

    let error = validate_args(&args).expect_err("ramp factor one is invalid");

    assert!(error.contains("--ramp-factor must be greater than 1"));
}

#[test]
fn thresholds_report_violating_run_label() {
    let mut args = test_args();
    args.max_failure_rate = Some(0.1);
    let metrics = sweep::RunMetrics {
        attempted: 10,
        failed: 2,
        throughput_ops_sec: 100.0,
        cpu_ms: Some(10),
        peak_rss_kb: Some(1024),
        p95_us: 1_000,
        p99_us: 1_000,
        max_us: 1_000,
    };

    let error = sweep::enforce_thresholds(&args, &[("c2".to_string(), metrics)])
        .expect_err("failure rate threshold should fail");

    assert!(error.contains("c2"));
    assert!(error.contains("failure_rate"));
}

#[test]
fn trace_child_path_keeps_parent_trace_name() {
    let child_path = trace::child_trace_path(Path::new("/tmp/ironclaw-trace.jsonl"), 3);

    assert_eq!(
        child_path,
        Path::new("/tmp/ironclaw-trace.jsonl.child-3.jsonl")
    );
}

#[test]
fn trace_labeled_path_sanitizes_label() {
    let trace_path =
        trace::labeled_trace_path(Path::new("/tmp/ironclaw-trace.jsonl"), "ramp concurrency/8");

    assert_eq!(
        trace_path,
        Path::new("/tmp/ironclaw-trace.jsonl.ramp_concurrency_8.jsonl")
    );
}

#[test]
fn progress_counters_drain_interval_latencies() {
    let counters = progress::ProgressCounters::new(true);

    counters.record(false, Duration::from_micros(10));
    counters.record(true, Duration::from_micros(20));

    assert_eq!(counters.snapshot().attempted, 2);
    assert_eq!(counters.snapshot().failed, 1);
    assert_eq!(counters.drain_interval_latencies_us(), vec![10, 20]);
    assert!(counters.drain_interval_latencies_us().is_empty());
}

#[test]
fn process_pressure_cpu_burn_generates_successful_samples() {
    let mut args = test_args();
    args.scenario = Scenario::CpuBurn;
    args.concurrency = 1;
    args.operations = 2;
    args.cpu_work_units = 10;

    let samples = process_pressure::run(&args).expect("cpu burn samples");

    assert_eq!(samples.len(), 2);
    assert!(samples.iter().all(|sample| sample.error.is_none()));
}

#[test]
fn process_pressure_memory_churn_generates_successful_samples() {
    let mut args = test_args();
    args.scenario = Scenario::MemoryChurn;
    args.concurrency = 1;
    args.operations = 2;
    args.memory_bytes = 4096;

    let samples = process_pressure::run(&args).expect("memory churn samples");

    assert_eq!(samples.len(), 2);
    assert!(samples.iter().all(|sample| sample.error.is_none()));
}

#[test]
fn uniform_model_latency_is_deterministic_and_bounded() {
    let mut args = test_args();
    args.run_id = Some("latency-test".to_string());
    args.model_latency_ms = 100;
    args.model_latency_profile = ModelLatencyProfile::Uniform;
    args.model_latency_jitter_ms = 50;

    let first = user_turn::synthetic_model_wait_ms(&args, 2, 3);
    let second = user_turn::synthetic_model_wait_ms(&args, 2, 3);

    assert_eq!(first, second);
    assert!((100..=150).contains(&first));
}

#[test]
fn tail_spike_model_latency_spikes_every_nth_operation() {
    let mut args = test_args();
    args.operations = 10;
    args.model_latency_ms = 100;
    args.model_latency_profile = ModelLatencyProfile::TailSpike;
    args.model_latency_spike_every = 3;
    args.model_latency_spike_ms = 900;

    assert_eq!(user_turn::synthetic_model_wait_ms(&args, 0, 1), 100);
    assert_eq!(user_turn::synthetic_model_wait_ms(&args, 0, 2), 900);
}

#[test]
fn provider_model_latency_requires_mixed_user_session() {
    let matches = Args::command()
        .try_get_matches_from([
            "ironclaw_stress",
            "--backend",
            "libsql",
            "--scenario",
            "chat-turn",
            "--model-latency-source",
            "provider",
        ])
        .expect("parse args");

    let args = parse_args_from_matches(&matches).expect("parse stress args");
    let error = validate_args(&args).expect_err("provider mode validation");

    assert!(
        error.contains("--model-latency-source provider requires --scenario mixed-user-session")
    );
}

#[test]
fn provider_model_latency_args_are_reported() {
    let args = parse_test_args([
        "ironclaw_stress",
        "--backend",
        "libsql",
        "--scenario",
        "mixed-user-session",
        "--model-latency-source",
        "provider",
        "--provider-model",
        "gpt-test",
        "--provider-max-tokens",
        "8",
    ]);

    assert_eq!(args.model_latency_source, ModelLatencySource::Provider);
    assert_eq!(args.provider_model.as_deref(), Some("gpt-test"));
    assert_eq!(args.provider_max_tokens, 8);
}

#[test]
fn synthetic_tool_failure_cadence_is_deterministic() {
    let mut args = test_args();
    args.operations = 10;
    args.tool_calls_per_turn = 3;
    args.tool_failure_every = 4;

    assert!(!user_turn::synthetic_tool_failed(&args, 0, 0, 0));
    assert!(user_turn::synthetic_tool_failed(&args, 0, 1, 0));
}

#[test]
fn failure_causes_are_grouped_by_bucket_and_stage() {
    let samples = vec![
        Sample {
            latency: Duration::from_micros(10),
            error: Some("turn_thread_busy".to_string()),
            failure: Some(FailureCause::new(
                "turn_thread_busy",
                "submit_turn",
                "thread already has an active run",
            )),
            stages: None,
        },
        Sample {
            latency: Duration::from_micros(20),
            error: Some("turn_thread_busy".to_string()),
            failure: Some(FailureCause::new(
                "turn_thread_busy",
                "mark_rejected_busy",
                "ignored alternate detail",
            )),
            stages: None,
        },
    ];

    let causes = summarize_failure_causes(&samples);
    let busy = causes
        .get("turn_thread_busy")
        .expect("busy failure summary");

    assert_eq!(busy.count, 2);
    assert_eq!(busy.stages["submit_turn"], 1);
    assert_eq!(busy.stages["mark_rejected_busy"], 1);
    assert_eq!(busy.sample_detail, "thread already has an active run");
}

#[test]
fn operation_attribution_groups_user_turn_stage_durations() {
    let samples = vec![Sample {
        latency: Duration::from_micros(50),
        error: None,
        failure: None,
        stages: Some(UserTurnStageDurations {
            ensure_thread: Some(Duration::from_micros(1)),
            accept_inbound: Some(Duration::from_micros(2)),
            submit_turn: Some(Duration::from_micros(3)),
            claim_run: Some(Duration::from_micros(4)),
            complete_run: Some(Duration::from_micros(5)),
            load_context: Some(Duration::from_micros(6)),
            resource_reserve: Some(Duration::from_micros(7)),
            resource_reconcile: Some(Duration::from_micros(8)),
            resource_release: Some(Duration::from_micros(9)),
            model_wait: Some(Duration::from_micros(10)),
            tool_wait: Some(Duration::from_micros(11)),
            append_assistant: Some(Duration::from_micros(12)),
            finalize_assistant: Some(Duration::from_micros(13)),
            append_tool_result: Some(Duration::from_micros(14)),
            append_tool_preview: Some(Duration::from_micros(15)),
            update_assistant_draft: Some(Duration::from_micros(16)),
            ..UserTurnStageDurations::default()
        }),
    }];

    let attribution =
        summary::summarize_user_turn_operation_attribution(&samples).expect("attribution summary");

    assert_eq!(attribution.thread_store_writes.count, 1);
    assert_eq!(attribution.thread_store_writes.latency.p95_us, 73);
    assert_eq!(attribution.turn_store.latency.p95_us, 12);
    assert_eq!(attribution.context_reads.latency.p95_us, 6);
    assert_eq!(attribution.resource_governor.latency.p95_us, 24);
    assert_eq!(attribution.synthetic_wait.latency.p95_us, 21);
}

#[test]
fn human_summary_includes_stage_latency_and_failure_tables() {
    let summary = run_summary_with_bottlenecks();

    let rendered = human::render_run_summary(&summary);

    assert!(rendered.contains("Operation attribution"));
    assert!(rendered.contains("thread_store_writes"));
    assert!(rendered.contains("model_tool_wait"));
    assert!(rendered.contains("Stage latency"));
    assert!(rendered.contains("submit_turn"));
    assert!(rendered.contains("resource_reserve"));
    assert!(rendered.contains("model_wait"));
    assert!(rendered.contains("append_tool_result"));
    assert!(rendered.contains("Prefill"));
    assert!(rendered.contains("DB probe"));
    assert!(rendered.contains("libsql_file"));
    assert!(rendered.contains("Failure causes"));
    assert!(rendered.contains("turn_thread_busy"));
}

#[test]
fn human_summary_places_db_probe_errors_in_matching_columns() {
    let mut summary = run_summary_with_bottlenecks();
    summary.db_probe = Some(db_probe::DbProbeSummary {
        before: db_probe::DbProbeSnapshot {
            error: Some("before_failed".to_string()),
            ..db_probe::DbProbeSnapshot::default()
        },
        after: db_probe::DbProbeSnapshot {
            error: Some("after_failed".to_string()),
            ..db_probe::DbProbeSnapshot::default()
        },
        delta: db_probe::DbProbeDelta::default(),
    });

    let rendered = human::render_run_summary(&summary);
    let before_line = rendered
        .lines()
        .find(|line| line.contains("before_error"))
        .expect("before error line");
    let after_line = rendered
        .lines()
        .find(|line| line.contains("after_error"))
        .expect("after error line");

    assert_eq!(
        before_line.split_whitespace().collect::<Vec<_>>(),
        ["before_error", "before_failed", "-", "-"]
    );
    assert_eq!(
        after_line.split_whitespace().collect::<Vec<_>>(),
        ["after_error", "-", "after_failed", "-"]
    );
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_probe_error_redacts_resolved_url() {
    let url = "postgresql://postgres:secret@localhost:5432/app";
    let error = format!("connection failed for {url}");

    let sanitized = db_probe::sanitize_postgres_error(url, error);

    assert!(sanitized.contains("postgresql://<redacted>@localhost:5432/app"));
    assert!(!sanitized.contains("secret"));
}

#[test]
fn bottleneck_report_identifies_failure_stage_and_db_growth() {
    let args = test_args();
    let captured = capture::CapturedRun::Single(Box::new(run_summary_with_bottlenecks()));

    let rendered = analysis::render_bottleneck_report(&args, "run", &captured);

    assert!(rendered.contains("Bottleneck analysis"));
    assert!(rendered.contains("failure_rate"));
    assert!(rendered.contains("turn_thread_busy"));
    assert!(rendered.contains("top_stage_p95"));
    assert!(rendered.contains("top_operation_group"));
    assert!(rendered.contains("model_tool_wait"));
    assert!(rendered.contains("model_wait"));
    assert!(rendered.contains("libsql_growth"));
}

#[test]
fn bottleneck_report_surfaces_missing_trace_file() {
    let mut args = test_args();
    args.trace_jsonl = Some(PathBuf::from("/tmp/ironclaw-missing-trace-test.jsonl"));
    let _ = std::fs::remove_file(args.trace_jsonl.as_ref().expect("trace path"));
    let captured = capture::CapturedRun::Single(Box::new(run_summary_with_bottlenecks()));

    let rendered = analysis::render_bottleneck_report(&args, "run", &captured);

    assert!(rendered.contains("trace_read_error"));
    assert!(rendered.contains("ironclaw-missing-trace-test.jsonl"));
}

#[test]
fn comparison_report_flags_single_run_regressions() {
    let baseline_path = temp_compare_path("single");
    let baseline = serde_json::json!({
        "attempted": 100,
        "failed": 0,
        "throughput_ops_sec": 100.0,
        "latency": {
            "p95_us": 1_000,
            "p99_us": 2_000,
            "max_us": 3_000
        },
        "process": {
            "delta_cpu_ms": 100,
            "peak_rss_kb": 1_000
        }
    });
    std::fs::write(&baseline_path, baseline.to_string()).expect("write baseline");
    let current = serde_json::json!({
        "attempted": 100,
        "failed": 5,
        "throughput_ops_sec": 70.0,
        "latency": {
            "p95_us": 2_000,
            "p99_us": 3_000,
            "max_us": 4_000
        },
        "process": {
            "delta_cpu_ms": 140,
            "peak_rss_kb": 1_400
        }
    });

    let rendered =
        compare::render_comparison_report(&baseline_path, &current).expect("comparison report");

    assert!(rendered.contains("Comparison report"));
    assert!(rendered.contains("throughput regression"));
    assert!(rendered.contains("p95 regression"));
    assert!(rendered.contains("failure-rate regression"));
    assert!(rendered.contains("CPU regression"));
    assert!(rendered.contains("RSS regression"));
    let _ = std::fs::remove_file(baseline_path);
}

#[test]
fn comparison_report_accepts_jsonl_suite() {
    let baseline_path = temp_compare_path("jsonl");
    let baseline = [
        serde_json::json!({
            "label": "c1",
            "metrics": {
                "attempted": 10,
                "failed": 0,
                "throughput_ops_sec": 10.0,
                "p95_us": 1_000,
                "p99_us": 1_000,
                "max_us": 1_000
            }
        }),
        serde_json::json!({
            "label": "c2",
            "metrics": {
                "attempted": 10,
                "failed": 0,
                "throughput_ops_sec": 20.0,
                "p95_us": 1_000,
                "p99_us": 1_000,
                "max_us": 1_000
            }
        }),
    ]
    .into_iter()
    .map(|value| value.to_string())
    .collect::<Vec<_>>()
    .join("\n");
    std::fs::write(&baseline_path, baseline).expect("write baseline jsonl");
    let current = serde_json::json!({
        "runs": [
            {
                "label": "c1",
                "metrics": {
                    "attempted": 10,
                    "failed": 0,
                    "throughput_ops_sec": 8.0,
                    "p95_us": 1_500,
                    "p99_us": 1_500,
                    "max_us": 1_500
                }
            },
            {
                "label": "c2",
                "metrics": {
                    "attempted": 10,
                    "failed": 0,
                    "throughput_ops_sec": 20.0,
                    "p95_us": 1_000,
                    "p99_us": 1_000,
                    "max_us": 1_000
                }
            }
        ]
    });

    let rendered =
        compare::render_comparison_report(&baseline_path, &current).expect("comparison report");

    assert!(rendered.contains("c1"));
    assert!(rendered.contains("c2"));
    assert!(rendered.contains("c1 throughput regression"));
    assert!(rendered.contains("c1 p95 regression"));
    let _ = std::fs::remove_file(baseline_path);
}

#[test]
fn comparison_report_compares_single_runs_with_different_labels() {
    let baseline_path = temp_compare_path("single-jsonl");
    let baseline = serde_json::json!({
        "label": "baseline-point",
        "metrics": {
            "attempted": 10,
            "failed": 0,
            "throughput_ops_sec": 10.0,
            "p95_us": 1_000,
            "p99_us": 1_000,
            "max_us": 1_000
        }
    });
    std::fs::write(&baseline_path, baseline.to_string()).expect("write baseline jsonl");
    let current = serde_json::json!({
        "attempted": 10,
        "failed": 0,
        "throughput_ops_sec": 5.0,
        "latency": {
            "p95_us": 1_000,
            "p99_us": 1_000,
            "max_us": 1_000
        }
    });

    let rendered =
        compare::render_comparison_report(&baseline_path, &current).expect("comparison report");

    assert!(rendered.contains("baseline-point -> run"));
    assert!(rendered.contains("throughput regression"));
    assert!(!rendered.contains("missing_in_baseline"));
    assert!(!rendered.contains("missing_in_current"));
    let _ = std::fs::remove_file(baseline_path);
}

fn run_summary_with_bottlenecks() -> RunSummary {
    let mut errors = std::collections::BTreeMap::new();
    errors.insert("turn_thread_busy".to_string(), 1);
    let mut cause_stages = std::collections::BTreeMap::new();
    cause_stages.insert("submit_turn".to_string(), 1);
    let mut failure_causes = std::collections::BTreeMap::new();
    failure_causes.insert(
        "turn_thread_busy".to_string(),
        FailureCauseSummary {
            count: 1,
            stages: cause_stages,
            sample_detail: "thread already has an active run".to_string(),
        },
    );
    RunSummary {
        backend: Backend::Libsql,
        preset: Some(StressPreset::ToolHeavy),
        scenario: Scenario::MixedUserSession,
        run_id: "run".to_string(),
        target: "libsql://<redacted-local-path>".to_string(),
        child_index: None,
        processes: 1,
        concurrency: 1,
        operations_per_thread: 1,
        duration_seconds: 0,
        warmup_seconds: 0,
        trace_jsonl_enabled: false,
        trace_interval_seconds: 1,
        users: 1,
        active_thread_count: 1,
        tenants: 1,
        prefill_threads: 1,
        prefill_turns_per_thread: 2,
        prefill_concurrency: 1,
        model_latency_ms: 0,
        model_latency_source: ModelLatencySource::Synthetic,
        provider_model: None,
        provider_max_tokens: 16,
        model_latency_profile: ModelLatencyProfile::Fixed,
        model_latency_jitter_ms: 0,
        model_latency_spike_every: 0,
        model_latency_spike_ms: 0,
        user_message_bytes: 0,
        assistant_message_bytes: 0,
        context_max_messages: 20,
        context_growth_turns_per_operation: 4,
        tool_calls_per_turn: 2,
        tool_latency_ms: 0,
        tool_output_bytes: 1024,
        tool_failure_every: 0,
        attempted: 1,
        succeeded: 0,
        failed: 1,
        duration_ms: 1,
        throughput_ops_sec: 1.0,
        latency: latency(1_000),
        process: ProcessMetrics::default(),
        db_probe: Some(db_probe_summary()),
        prefill: Some(prefill_summary()),
        operation_attribution: Some(user_turn::UserTurnOperationAttributionSummary {
            thread_store_writes: attribution(1, 10_000),
            context_reads: empty_attribution(),
            turn_store: attribution(1, 2_000),
            resource_governor: attribution(1, 3_000),
            synthetic_wait: attribution(1, 1_000_000),
        }),
        stage_latency: Some(UserTurnStageLatencySummary {
            ensure_thread: empty_stage(),
            accept_inbound: empty_stage(),
            submit_turn: stage(1, 2_000),
            mark_submitted: empty_stage(),
            mark_rejected_busy: empty_stage(),
            claim_run: empty_stage(),
            append_assistant: empty_stage(),
            finalize_assistant: empty_stage(),
            complete_run: empty_stage(),
            load_context: empty_stage(),
            resource_reserve: stage(1, 3_000),
            model_wait: stage(1, 1_000_000),
            tool_wait: stage(1, 4_000),
            append_tool_result: stage(1, 5_000),
            append_tool_preview: stage(1, 6_000),
            update_assistant_draft: stage(1, 7_000),
            resource_reconcile: empty_stage(),
            resource_release: empty_stage(),
        }),
        errors,
        failure_causes,
    }
}

fn test_args() -> Args {
    Args {
        backend: Backend::Libsql,
        preset: None,
        suite: None,
        processes: 1,
        concurrency: 2,
        operations: 3,
        duration_seconds: 0,
        warmup_seconds: 0,
        users: 4,
        active_thread_count: 0,
        tenants: 2,
        prefill_threads: 0,
        prefill_turns_per_thread: 0,
        prefill_concurrency: 4,
        scenario: Scenario::ReserveRelease,
        run_id: None,
        libsql_path: None,
        postgres_url: None,
        postgres_pool_size: 4,
        progress_interval_seconds: 0,
        human_read: false,
        bottleneck_report: false,
        compare_json: None,
        ramp_concurrency: None,
        ramp_users: None,
        ramp_factor: 2,
        sweep_concurrency: Vec::new(),
        sweep_users: Vec::new(),
        sweep_active_thread_count: Vec::new(),
        sweep_model_latency_ms: Vec::new(),
        sweep_user_message_bytes: Vec::new(),
        sweep_assistant_message_bytes: Vec::new(),
        sweep_context_max_messages: Vec::new(),
        sweep_context_growth_turns_per_operation: Vec::new(),
        sweep_tool_calls_per_turn: Vec::new(),
        sweep_tool_output_bytes: Vec::new(),
        repetitions: 1,
        output_jsonl: None,
        trace_jsonl: None,
        trace_interval_seconds: 1,
        max_failure_rate: None,
        max_p95_ms: None,
        min_throughput: None,
        max_rss_mb: None,
        max_cpu_ms: None,
        model_latency_ms: 0,
        model_latency_source: ModelLatencySource::Synthetic,
        provider_model: None,
        provider_max_tokens: 16,
        model_latency_profile: ModelLatencyProfile::Fixed,
        model_latency_jitter_ms: 0,
        model_latency_spike_every: 0,
        model_latency_spike_ms: 0,
        user_message_bytes: 0,
        assistant_message_bytes: 0,
        context_max_messages: 20,
        context_growth_turns_per_operation: 4,
        tool_calls_per_turn: 2,
        tool_latency_ms: 0,
        tool_output_bytes: 1024,
        tool_failure_every: 0,
        span_log_failures: false,
        slow_span_threshold_ms: 0,
        span_sample_limit: 100,
        cpu_work_units: 10,
        memory_bytes: 4096,
        memory_hold_ms: 0,
        child_index: None,
        warmup_phase: false,
        suite_case_label: None,
    }
}

fn parse_test_args<const N: usize>(args: [&str; N]) -> Args {
    let matches = Args::command()
        .try_get_matches_from(args)
        .expect("parse test args");
    parse_args_from_matches(&matches).expect("build args")
}

fn db_probe_summary() -> db_probe::DbProbeSummary {
    db_probe::DbProbeSummary {
        before: db_probe::DbProbeSnapshot {
            libsql_file_bytes: Some(1024),
            ..db_probe::DbProbeSnapshot::default()
        },
        after: db_probe::DbProbeSnapshot {
            libsql_file_bytes: Some(2048),
            ..db_probe::DbProbeSnapshot::default()
        },
        delta: db_probe::DbProbeDelta {
            libsql_file_bytes: Some(1024),
            ..db_probe::DbProbeDelta::default()
        },
    }
}

fn prefill_summary() -> user_turn::PrefillSummary {
    user_turn::PrefillSummary {
        threads: 1,
        turns_per_thread: 2,
        concurrency: 1,
        attempted: 2,
        succeeded: 2,
        failed: 0,
        duration_ms: 10,
        throughput_ops_sec: 200.0,
        latency: latency(1_000),
        errors: std::collections::BTreeMap::new(),
    }
}

fn stage(count: u64, p50_us: u128) -> user_turn::StageLatencySummary {
    user_turn::StageLatencySummary {
        count,
        latency: latency(p50_us),
    }
}

fn empty_stage() -> user_turn::StageLatencySummary {
    stage(0, 0)
}

fn attribution(count: u64, p50_us: u128) -> user_turn::OperationAttributionSummary {
    user_turn::OperationAttributionSummary {
        count,
        latency: latency(p50_us),
    }
}

fn empty_attribution() -> user_turn::OperationAttributionSummary {
    attribution(0, 0)
}

fn latency(p50_us: u128) -> LatencySummary {
    LatencySummary {
        min_us: p50_us,
        p50_us,
        p95_us: p50_us,
        p99_us: p50_us,
        max_us: p50_us,
    }
}

fn temp_compare_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "ironclaw-compare-{label}-{}.json",
        std::process::id()
    ))
}
