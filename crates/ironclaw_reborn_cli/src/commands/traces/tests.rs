use super::*;
use crate::cli::Cli;
use crate::commands::Command;
use ironclaw_reborn_traces::contribution::{
    ConsentMetadata, ContributorMetadata, DETERMINISTIC_REDACTION_PIPELINE_VERSION,
    IronclawTraceMetadata, OutcomeMetadata, PrivacyMetadata, ReplayMetadata, ResidualPiiRisk,
    TRACE_CONTRIBUTION_POLICY_VERSION, TRACE_CONTRIBUTION_SCHEMA_VERSION, TraceCard,
    TraceValueCard, ValueMetadata,
};
use clap::Parser;

fn unwrap_traces_command(cli: Cli) -> TracesSubcommand {
    let Command::Traces(command) = cli.command else {
        panic!("expected traces command");
    };
    command.command
}

fn parse_cli_result<const N: usize>(args: [&'static str; N]) -> Result<Cli, clap::Error> {
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || Cli::try_parse_from(args))
        .expect("parser thread should spawn")
        .join()
        .expect("parser thread should not panic")
}

fn parse_cli<const N: usize>(args: [&'static str; N]) -> Cli {
    parse_cli_result(args).expect("CLI args should parse")
}

struct TracePolicyFileRestore {
    path: PathBuf,
    previous: Option<String>,
}

impl TracePolicyFileRestore {
    fn new(path: PathBuf) -> Self {
        Self {
            previous: std::fs::read_to_string(&path).ok(),
            path,
        }
    }
}

impl Drop for TracePolicyFileRestore {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent).expect("policy parent restores");
            }
            std::fs::write(&self.path, previous).expect("policy restores");
        } else if self.path.exists() {
            std::fs::remove_file(&self.path).expect("test policy removes");
        }
    }
}

#[test]
fn trace_scope_arg_maps_to_consent_scope() {
    assert_eq!(
        ConsentScope::from(TraceScopeArg::BenchmarkOnly),
        ConsentScope::BenchmarkOnly
    );
}

#[test]
fn redaction_summary_handles_empty_and_counts() {
    assert_eq!(
        redaction_summary(&BTreeMap::new()),
        "no deterministic redactions"
    );

    let mut counts = BTreeMap::new();
    counts.insert("local_path".to_string(), 2);
    counts.insert("secret".to_string(), 1);
    assert_eq!(redaction_summary(&counts), "2 local_path, 1 secret");
}

fn trace_queue_policy_fixture() -> StandingTraceContributionPolicy {
    StandingTraceContributionPolicy {
        enabled: true,
        ingestion_endpoint: Some("https://trace.example/internal/v1/traces".to_string()),
        ..StandingTraceContributionPolicy::default()
    }
}

fn trace_queue_envelope_fixture(
    message_text_included: bool,
    tool_payloads_included: bool,
) -> TraceContributionEnvelope {
    TraceContributionEnvelope {
        schema_version: TRACE_CONTRIBUTION_SCHEMA_VERSION.to_string(),
        trace_id: Uuid::new_v4(),
        submission_id: Uuid::new_v4(),
        created_at: chrono::Utc::now(),
        ironclaw: IronclawTraceMetadata {
            version: env!("CARGO_PKG_VERSION").to_string(),
            engine_version: None,
            feature_flags: BTreeMap::new(),
            channel: TraceChannel::Cli,
            model_name: None,
        },
        consent: ConsentMetadata {
            policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
            scopes: vec![ConsentScope::DebuggingEvaluation],
            message_text_included,
            tool_payloads_included,
            revocable: true,
        },
        contributor: ContributorMetadata {
            pseudonymous_contributor_id: None,
            tenant_scope_ref: None,
            credit_account_ref: None,
            revocation_handle: Uuid::new_v4(),
        },
        privacy: PrivacyMetadata {
            redaction_pipeline_version: DETERMINISTIC_REDACTION_PIPELINE_VERSION.to_string(),
            redaction_counts: BTreeMap::new(),
            privacy_filter_summary: None,
            pii_labels_present: Vec::new(),
            residual_pii_risk: ResidualPiiRisk::Low,
            redaction_hash: "sha256:test".to_string(),
            warnings: Vec::new(),
        },
        events: Vec::new(),
        outcome: OutcomeMetadata::default(),
        replay: ReplayMetadata {
            replayable: false,
            required_tools: Vec::new(),
            tool_manifest_hashes: BTreeMap::new(),
            expected_assertions: Vec::new(),
            replay_notes: Vec::new(),
        },
        embedding_analysis: None,
        value: ValueMetadata::default(),
        trace_card: TraceCard::default(),
        value_card: TraceValueCard::default(),
        hindsight: None,
        training_dynamics: None,
        process_evaluation: None,
    }
}

#[test]
fn cli_trace_upload_preflight_keeps_preview_local_but_gates_queue_intents() {
    let envelope = trace_queue_envelope_fixture(true, true);
    let policy = StandingTraceContributionPolicy::default();

    preflight_cli_trace_envelope_upload(
        &policy,
        TraceContributionAcceptance::PreviewOnly,
        &envelope,
    )
    .expect("local preview remains available before opt-in");
    let error = preflight_cli_trace_envelope_upload(
        &policy,
        TraceContributionAcceptance::QueueFromPreview,
        &envelope,
    )
    .expect_err("queueing requires standing opt-in");

    assert!(error.to_string().contains("opt-in is disabled"));
}

#[test]
fn cli_enqueue_rejects_capture_fields_disallowed_by_policy_before_write() {
    let dir = tempfile::tempdir().expect("tempdir");
    let policy = trace_queue_policy_fixture();
    let envelope = trace_queue_envelope_fixture(true, false);
    let path = dir.path().join(format!("{}.json", envelope.submission_id));

    let error = enqueue_envelope_to_dir_with_policy(
        &envelope,
        &policy,
        TraceContributionAcceptance::QueueFromPreview,
        dir.path(),
    )
    .expect_err("message text requires standing opt-in capture permission");

    assert!(error.to_string().contains("message text upload"));
    assert!(!path.exists(), "rejected envelopes must not be queued");
}

#[test]
fn cli_enqueue_rejects_tool_payloads_disallowed_by_policy_before_write() {
    let dir = tempfile::tempdir().expect("tempdir");
    let policy = trace_queue_policy_fixture();
    let envelope = trace_queue_envelope_fixture(false, true);
    let path = dir.path().join(format!("{}.json", envelope.submission_id));

    let error = enqueue_envelope_to_dir_with_policy(
        &envelope,
        &policy,
        TraceContributionAcceptance::QueueFromPreview,
        dir.path(),
    )
    .expect_err("tool payloads require standing opt-in capture permission");

    assert!(error.to_string().contains("tool payload upload"));
    assert!(!path.exists(), "rejected envelopes must not be queued");
}

#[test]
fn cli_enqueue_accepts_policy_matching_envelope_capture() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut policy = trace_queue_policy_fixture();
    policy.include_message_text = true;
    let envelope = trace_queue_envelope_fixture(true, false);

    let path = enqueue_envelope_to_dir_with_policy(
        &envelope,
        &policy,
        TraceContributionAcceptance::QueueFromPreview,
        dir.path(),
    )
    .expect("matching standing policy should queue envelope");

    assert!(path.exists());
}

#[test]
fn list_submissions_summary_flag_parses_through_cli() {
    let cli = parse_cli(["ironclaw-reborn", "traces", "list-submissions", "--summary"]);

    let TracesSubcommand::ListSubmissions { json, summary } = unwrap_traces_command(cli) else {
        panic!("expected traces list-submissions command");
    };

    assert!(!json);
    assert!(summary);
}

#[test]
fn credit_notice_flags_parse_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "credit",
        "--notice",
        "--notice-scope",
        "tenant-a:user-alice",
    ]);

    let TracesSubcommand::Credit {
        json,
        notice,
        notice_scope,
        ack,
        snooze_hours,
    } = unwrap_traces_command(cli)
    else {
        panic!("expected traces credit command");
    };

    assert!(!json);
    assert!(notice);
    assert_eq!(notice_scope.as_deref(), Some("tenant-a:user-alice"));
    assert!(!ack);
    assert_eq!(snooze_hours, None);
}

#[test]
fn credit_notice_action_flags_parse_through_cli() {
    let ack_cli = parse_cli(["ironclaw-reborn", "traces", "credit", "--notice", "--ack"]);
    let TracesSubcommand::Credit {
        notice: ack_notice,
        ack,
        snooze_hours,
        ..
    } = unwrap_traces_command(ack_cli)
    else {
        panic!("expected traces credit command");
    };
    assert!(ack_notice);
    assert!(ack);
    assert_eq!(snooze_hours, None);

    let snooze_cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "credit",
        "--notice",
        "--snooze-hours",
        "24",
    ]);
    let TracesSubcommand::Credit {
        notice: snooze_notice,
        ack,
        snooze_hours,
        ..
    } = unwrap_traces_command(snooze_cli)
    else {
        panic!("expected traces credit command");
    };
    assert!(snooze_notice);
    assert!(!ack);
    assert_eq!(snooze_hours, Some(24));
}

#[test]
fn queue_status_flags_parse_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "queue-status",
        "--json",
        "--scope",
        "tenant-a:user-alice",
    ]);

    let TracesSubcommand::QueueStatus { json, scope } = unwrap_traces_command(cli) else {
        panic!("expected traces queue-status command");
    };

    assert!(json);
    assert_eq!(scope.as_deref(), Some("tenant-a:user-alice"));
}

#[test]
fn opt_in_upload_claim_issuer_flags_parse_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "opt-in",
        "--endpoint",
        "https://trace.example/v1/traces",
        "--upload-token-issuer-url",
        "https://issuer.example/v1/trace-upload-claim",
        "--upload-token-issuer-allowed-hosts",
        "issuer.example",
        "--upload-token-audience",
        "trace-commons",
        "--upload-token-tenant-id",
        "tenant-a",
        "--upload-token-workload-token-env",
        "IRONCLAW_TRACE_WORKLOAD_TOKEN",
        "--upload-token-issuer-timeout-ms",
        "7000",
    ]);

    let TracesSubcommand::OptIn {
        upload_token_issuer_url,
        upload_token_issuer_allowed_hosts,
        upload_token_audience,
        upload_token_tenant_id,
        upload_token_workload_token_env,
        upload_token_issuer_timeout_ms,
        ..
    } = unwrap_traces_command(cli)
    else {
        panic!("expected traces opt-in command");
    };

    assert_eq!(
        upload_token_issuer_url.as_deref(),
        Some("https://issuer.example/v1/trace-upload-claim")
    );
    assert_eq!(upload_token_issuer_allowed_hosts, vec!["issuer.example"]);
    assert_eq!(upload_token_audience.as_deref(), Some("trace-commons"));
    assert_eq!(upload_token_tenant_id.as_deref(), Some("tenant-a"));
    assert_eq!(
        upload_token_workload_token_env.as_deref(),
        Some("IRONCLAW_TRACE_WORKLOAD_TOKEN")
    );
    assert_eq!(upload_token_issuer_timeout_ms, 7000);
}

#[test]
fn opt_in_invite_code_flag_parses_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "opt-in",
        "--endpoint",
        "https://trace.example/v1/traces",
        "--upload-token-issuer-url",
        "https://issuer.example/v1/trace-upload-claim",
        "--upload-token-issuer-allowed-hosts",
        "issuer.example",
        "--upload-token-invite-code",
        "INV-PILOT-001",
    ]);

    let TracesSubcommand::OptIn {
        upload_token_invite_code,
        ..
    } = unwrap_traces_command(cli)
    else {
        panic!("expected traces opt-in command");
    };

    assert_eq!(upload_token_invite_code.as_deref(), Some("INV-PILOT-001"));
}

#[test]
fn opt_in_invite_code_defaults_to_none_when_absent() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "opt-in",
        "--endpoint",
        "https://trace.example/v1/traces",
    ]);

    let TracesSubcommand::OptIn {
        upload_token_invite_code,
        ..
    } = unwrap_traces_command(cli)
    else {
        panic!("expected traces opt-in command");
    };

    assert!(upload_token_invite_code.is_none());
}

#[test]
fn opt_in_user_scope_flag_parses_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "opt-in",
        "--endpoint",
        "https://trace.example/v1/traces",
        "--user-scope",
        "tenant-a:user-alice",
    ]);

    let TracesSubcommand::OptIn { user_scope, .. } = unwrap_traces_command(cli) else {
        panic!("expected traces opt-in command");
    };

    assert_eq!(user_scope.as_deref(), Some("tenant-a:user-alice"));
}

#[test]
fn opt_in_writes_runtime_owner_policy_and_normalizes_selected_tools() {
    let runtime_scope = format!("trace-cli-runtime-scope-{}", Uuid::new_v4());
    let _global_policy_restore = TracePolicyFileRestore::new(policy_path());
    let _runtime_policy_restore = TracePolicyFileRestore::new(
        ironclaw_reborn_traces::contribution::trace_contribution_dir_for_scope(Some(&runtime_scope))
            .join("policy.json"),
    );

    opt_in(OptInOptions {
        endpoint: "https://trace.example.com/v1/traces".to_string(),
        user_scope: Some(runtime_scope.clone()),
        bearer_token_env: "TRACE_COMMONS_TEST_TOKEN".to_string(),
        upload_token_issuer_url: None,
        upload_token_issuer_allowed_hosts: Vec::new(),
        upload_token_audience: None,
        upload_token_tenant_id: None,
        upload_token_workload_token_env: None,
        upload_token_invite_code: None,
        upload_token_issuer_timeout_ms:
            ironclaw_reborn_traces::contribution::TRACE_UPLOAD_CLAIM_DEFAULT_TIMEOUT_MS,
        include_message_text: true,
        include_tool_payloads: false,
        scope: TraceScopeArg::DebuggingEvaluation,
        selected_tools: vec![" shell ".to_string(), " ".to_string(), "http".to_string()],
        allow_pii_review_bypass: true,
        min_submission_score: 0.2,
    })
    .expect("opt-in succeeds");

    let global = read_policy().expect("global policy reads");
    let scoped =
        read_trace_policy_for_scope(Some(&runtime_scope)).expect("scoped policy reads");
    assert!(global.enabled);
    assert!(scoped.enabled);
    assert_eq!(
        scoped.ingestion_endpoint.as_deref(),
        Some("https://trace.example.com/v1/traces")
    );
    assert_eq!(
        scoped.selected_tools,
        BTreeSet::from(["http".to_string(), "shell".to_string()])
    );
    assert_eq!(global.selected_tools, scoped.selected_tools);
}

#[test]
fn local_credit_summary_includes_delayed_ledger_totals() {
    let submission_id =
        Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("valid uuid");
    let trace_id = Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac02").expect("valid uuid");
    let summary = credit_summary(&[LocalSubmissionRecord {
        submission_id,
        trace_id,
        endpoint: Some("https://trace.example/internal/v1/traces".to_string()),
        status: LocalSubmissionStatus::Submitted,
        server_status: Some("accepted".to_string()),
        submitted_at: Some(chrono::Utc::now()),
        revoked_at: None,
        privacy_risk: "low".to_string(),
        redaction_counts: BTreeMap::new(),
        credit_points_pending: 1.0,
        credit_points_final: Some(2.25),
        credit_explanation: vec!["Accepted after privacy checks.".to_string()],
        credit_events: vec![
            TraceCreditEvent {
                event_id: Uuid::new_v4(),
                submission_id,
                contributor_pseudonym: "contributor-a".to_string(),
                kind: TraceCreditEventKind::Accepted,
                points_delta: 1.0,
                reason: "Accepted".to_string(),
                created_at: chrono::Utc::now(),
            },
            TraceCreditEvent {
                event_id: Uuid::new_v4(),
                submission_id,
                contributor_pseudonym: "contributor-a".to_string(),
                kind: TraceCreditEventKind::UsedForTrainingOrRanking,
                points_delta: 1.25,
                reason: "Process evaluation utility.".to_string(),
                created_at: chrono::Utc::now(),
            },
        ],
        last_credit_notice_at: None,
    }]);

    assert_eq!(summary.pending_credit, 1.0);
    assert_eq!(summary.final_credit, 2.25);
    assert_eq!(summary.delayed_credit_delta, 1.25);
    assert_eq!(summary.credit_events_total, 2);
}

#[test]
fn cli_credit_sync_resets_notice_when_explanation_changes_without_credit_delta() {
    let submission_id =
        Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac11").expect("valid uuid");
    let trace_id = Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac12").expect("valid uuid");
    let now = chrono::Utc::now();
    let mut records = vec![LocalSubmissionRecord {
        submission_id,
        trace_id,
        endpoint: Some("https://trace.example/internal/v1/traces".to_string()),
        status: LocalSubmissionStatus::Submitted,
        server_status: Some("accepted".to_string()),
        submitted_at: Some(now),
        revoked_at: None,
        privacy_risk: "low".to_string(),
        redaction_counts: BTreeMap::new(),
        credit_points_pending: 1.0,
        credit_points_final: Some(2.0),
        credit_explanation: vec!["Previous explanation.".to_string()],
        credit_events: Vec::new(),
        last_credit_notice_at: Some(now),
    }];
    let update = TraceSubmissionStatusUpdate {
        submission_id,
        trace_id,
        status: "accepted".to_string(),
        credit_points_pending: 1.0,
        credit_points_final: Some(2.0),
        credit_points_ledger: 0.0,
        credit_points_total: Some(2.0),
        explanation: vec!["Accepted after privacy checks.".to_string()],
        delayed_credit_explanations: vec![
            "Process evaluation changed the utility explanation.".to_string(),
        ],
    };

    let changed = apply_cli_submission_status_updates_to_records(
        &mut records,
        std::slice::from_ref(&update),
        now,
    );

    assert_eq!(changed, 1);
    assert!(records[0].last_credit_notice_at.is_none());
    assert_eq!(
        records[0].credit_explanation,
        vec![
            "Accepted after privacy checks.".to_string(),
            "Process evaluation changed the utility explanation.".to_string(),
        ]
    );
    assert_eq!(records[0].credit_events.len(), 1);
    assert_eq!(
        records[0].credit_events[0].kind,
        TraceCreditEventKind::CreditSynced
    );
    assert_eq!(records[0].credit_events[0].points_delta, 0.0);

    let unchanged = apply_cli_submission_status_updates_to_records(
        &mut records,
        std::slice::from_ref(&update),
        now,
    );

    assert_eq!(unchanged, 0);
    assert_eq!(records[0].credit_events.len(), 1);
}

#[test]
fn cli_credit_notice_message_includes_delayed_and_event_totals() {
    let message = credit_notice_message(&CreditSummary {
        submissions_total: 4,
        submissions_submitted: 2,
        submissions_revoked: 1,
        submissions_expired: 1,
        pending_credit: 3.5,
        final_credit: 2.0,
        delayed_credit_delta: 0.75,
        credit_events_total: 5,
        recent_explanations: Vec::new(),
    });

    assert!(message.contains("2 submitted"));
    assert!(message.contains("1 expired (4 total)"));
    assert!(message.contains("pending +3.50"));
    assert!(message.contains("final confirmed +2.00"));
    assert!(message.contains("delayed ledger +0.75"));
    assert!(message.contains("5 credit event(s) recorded"));
    assert!(message.contains("Delayed credit can change"));
}

