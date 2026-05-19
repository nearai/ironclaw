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
fn active_learning_review_queue_uses_lease_filter_query() {
    let url = trace_commons_api_url(
        "https://trace.example/internal/v1/traces",
        "/v1/review/active-learning",
        &[
            ("limit", "25".to_string()),
            ("lease_filter", "available".to_string()),
        ],
    )
    .expect("url builds");

    assert_eq!(
        url,
        "https://trace.example/internal/v1/review/active-learning?limit=25&lease_filter=available"
    );
}

#[test]
fn quarantine_list_lease_filter_parses_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "quarantine-list",
        "--endpoint",
        "https://trace.example/v1/traces",
        "--lease-filter",
        "mine",
    ]);

    let TracesSubcommand::QuarantineList { lease_filter, .. } = unwrap_traces_command(cli) else {
        panic!("expected traces quarantine-list command");
    };

    assert_eq!(lease_filter, Some(TraceReviewLeaseFilterArg::Mine));
}

#[test]
fn review_lease_claim_parses_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "review-lease-claim",
        "--endpoint",
        "https://trace.example/internal",
        "--submission-id",
        "018f2b7b-0c11-72fd-95c4-1f9f98feac01",
        "--lease-ttl-seconds",
        "900",
        "--review-due-at",
        "2026-04-26T12:00:00Z",
        "--bearer-token-env",
        "TRACE_COMMONS_REVIEWER_TOKEN",
        "--json",
    ]);

    let TracesSubcommand::ReviewLeaseClaim {
        endpoint,
        submission_id,
        lease_ttl_seconds,
        review_due_at,
        bearer_token_env,
        json,
    } = unwrap_traces_command(cli)
    else {
        panic!("expected traces review-lease-claim command");
    };

    assert_eq!(endpoint, "https://trace.example/internal");
    assert_eq!(
        submission_id,
        Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("uuid parses")
    );
    assert_eq!(lease_ttl_seconds, Some(900));
    assert_eq!(review_due_at.as_deref(), Some("2026-04-26T12:00:00Z"));
    assert_eq!(bearer_token_env, "TRACE_COMMONS_REVIEWER_TOKEN");
    assert!(json);
}

#[test]
fn review_lease_claim_next_parses_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "review-lease-claim-next",
        "--endpoint",
        "https://trace.example/internal",
        "--lease-ttl-seconds",
        "900",
        "--review-due-at",
        "2026-04-26T12:00:00Z",
        "--privacy-risk",
        "medium",
        "--bearer-token-env",
        "TRACE_COMMONS_REVIEWER_TOKEN",
        "--json",
    ]);

    let TracesSubcommand::ReviewLeaseClaimNext {
        endpoint,
        lease_ttl_seconds,
        review_due_at,
        privacy_risk,
        bearer_token_env,
        json,
    } = unwrap_traces_command(cli)
    else {
        panic!("expected traces review-lease-claim-next command");
    };

    assert_eq!(endpoint, "https://trace.example/internal");
    assert_eq!(lease_ttl_seconds, Some(900));
    assert_eq!(review_due_at.as_deref(), Some("2026-04-26T12:00:00Z"));
    assert_eq!(privacy_risk, Some(TracePrivacyRiskArg::Medium));
    assert_eq!(bearer_token_env, "TRACE_COMMONS_REVIEWER_TOKEN");
    assert!(json);
}

#[test]
fn review_lease_claim_batch_parses_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "review-lease-claim-batch",
        "--endpoint",
        "https://trace.example/internal",
        "--limit",
        "5",
        "--lease-ttl-seconds",
        "900",
        "--review-due-at",
        "2026-04-26T12:00:00Z",
        "--privacy-risk",
        "high",
        "--bearer-token-env",
        "TRACE_COMMONS_REVIEWER_TOKEN",
        "--json",
    ]);

    let TracesSubcommand::ReviewLeaseClaimBatch {
        endpoint,
        limit,
        lease_ttl_seconds,
        review_due_at,
        privacy_risk,
        bearer_token_env,
        json,
    } = unwrap_traces_command(cli)
    else {
        panic!("expected traces review-lease-claim-batch command");
    };

    assert_eq!(endpoint, "https://trace.example/internal");
    assert_eq!(limit, Some(5));
    assert_eq!(lease_ttl_seconds, Some(900));
    assert_eq!(review_due_at.as_deref(), Some("2026-04-26T12:00:00Z"));
    assert_eq!(privacy_risk, Some(TracePrivacyRiskArg::High));
    assert_eq!(bearer_token_env, "TRACE_COMMONS_REVIEWER_TOKEN");
    assert!(json);
}

#[test]
fn review_lease_release_parses_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "review-lease-release",
        "--endpoint",
        "https://trace.example/internal",
        "--submission-id",
        "018f2b7b-0c11-72fd-95c4-1f9f98feac01",
        "--bearer-token-env",
        "TRACE_COMMONS_REVIEWER_TOKEN",
    ]);

    let TracesSubcommand::ReviewLeaseRelease {
        endpoint,
        submission_id,
        bearer_token_env,
        json,
    } = unwrap_traces_command(cli)
    else {
        panic!("expected traces review-lease-release command");
    };

    assert_eq!(endpoint, "https://trace.example/internal");
    assert_eq!(
        submission_id,
        Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("uuid parses")
    );
    assert_eq!(bearer_token_env, "TRACE_COMMONS_REVIEWER_TOKEN");
    assert!(!json);
}

#[test]
fn review_lease_uses_submission_route_from_ingest_endpoint() {
    let submission_id =
        Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("uuid parses");
    let url = trace_commons_api_url(
        "https://trace.example/internal/v1/traces",
        &format!("/v1/review/{submission_id}/lease"),
        &[],
    )
    .expect("url builds");

    assert_eq!(
        url,
        "https://trace.example/internal/v1/review/018f2b7b-0c11-72fd-95c4-1f9f98feac01/lease"
    );
}

#[test]
fn review_lease_claim_next_uses_ingest_endpoint() {
    let url = trace_commons_api_url(
        "https://trace.example/internal/v1/traces",
        "/v1/review/leases/claim-next",
        &[],
    )
    .expect("url builds");

    assert_eq!(
        url,
        "https://trace.example/internal/v1/review/leases/claim-next"
    );
}

#[test]
fn review_lease_claim_batch_uses_ingest_endpoint() {
    let url = trace_commons_api_url(
        "https://trace.example/internal/v1/traces",
        "/v1/review/leases/claim-batch",
        &[],
    )
    .expect("url builds");

    assert_eq!(
        url,
        "https://trace.example/internal/v1/review/leases/claim-batch"
    );
}

#[test]
fn review_lease_claim_body_omits_absent_optional_fields() {
    let body = trace_commons_review_lease_claim_body(None, None).expect("body builds");

    assert!(body.as_object().expect("body is object").is_empty());
}

#[test]
fn review_lease_claim_body_includes_optional_fields() {
    let body = trace_commons_review_lease_claim_body(
        Some(900),
        Some("2026-04-26T12:00:00Z".to_string()),
    )
    .expect("body builds");

    assert_eq!(body["lease_ttl_seconds"], 900);
    assert_eq!(body["review_due_at"], "2026-04-26T12:00:00Z");
}

#[test]
fn review_lease_claim_body_rejects_invalid_optional_fields() {
    let ttl_error = trace_commons_review_lease_claim_body(Some(0), None)
        .expect_err("non-positive TTL is rejected");
    assert!(ttl_error.to_string().contains("greater than 0"));

    let due_at_error =
        trace_commons_review_lease_claim_body(None, Some("tomorrow-ish".to_string()))
            .expect_err("non-RFC3339 due timestamp is rejected");
    assert!(due_at_error.to_string().contains("RFC3339"));
}

#[test]
fn review_lease_claim_next_body_includes_optional_fields() {
    let body = trace_commons_review_lease_claim_next_body(
        Some(900),
        Some("2026-04-26T12:00:00Z".to_string()),
        Some(TracePrivacyRiskArg::High),
    )
    .expect("body builds");

    assert_eq!(body["lease_ttl_seconds"], 900);
    assert_eq!(body["review_due_at"], "2026-04-26T12:00:00Z");
    assert_eq!(body["privacy_risk"], "high");
}

#[test]
fn review_lease_claim_next_body_rejects_invalid_optional_fields() {
    let ttl_error = trace_commons_review_lease_claim_next_body(Some(0), None, None)
        .expect_err("non-positive TTL is rejected");
    assert!(ttl_error.to_string().contains("greater than 0"));

    let due_at_error = trace_commons_review_lease_claim_next_body(
        None,
        Some("tomorrow-ish".to_string()),
        None,
    )
    .expect_err("non-RFC3339 due timestamp is rejected");
    assert!(due_at_error.to_string().contains("RFC3339"));
}

#[test]
fn review_lease_claim_batch_body_includes_optional_fields() {
    let body = trace_commons_review_lease_claim_batch_body(
        Some(5),
        Some(900),
        Some("2026-04-26T12:00:00Z".to_string()),
        Some(TracePrivacyRiskArg::High),
    )
    .expect("body builds");

    assert_eq!(body["limit"], 5);
    assert_eq!(body["lease_ttl_seconds"], 900);
    assert_eq!(body["review_due_at"], "2026-04-26T12:00:00Z");
    assert_eq!(body["privacy_risk"], "high");
}

#[test]
fn review_lease_claim_batch_body_rejects_invalid_optional_fields() {
    let limit_error = trace_commons_review_lease_claim_batch_body(Some(0), None, None, None)
        .expect_err("zero limit is rejected");
    assert!(limit_error.to_string().contains("greater than 0"));

    let ttl_error = trace_commons_review_lease_claim_batch_body(Some(5), Some(0), None, None)
        .expect_err("non-positive TTL is rejected");
    assert!(ttl_error.to_string().contains("greater than 0"));

    let due_at_error = trace_commons_review_lease_claim_batch_body(
        Some(5),
        None,
        Some("tomorrow-ish".to_string()),
        None,
    )
    .expect_err("non-RFC3339 due timestamp is rejected");
    assert!(due_at_error.to_string().contains("RFC3339"));
}

#[test]
fn ranker_training_candidates_use_ingest_endpoint() {
    let url = trace_commons_api_url(
        "https://trace.example/internal",
        "/v1/ranker/training-candidates",
        &[
            ("consent_scope", "ranking-training".to_string()),
            ("status", "accepted".to_string()),
        ],
    )
    .expect("url builds");

    assert_eq!(
        url,
        "https://trace.example/internal/v1/ranker/training-candidates?consent_scope=ranking-training&status=accepted"
    );
}

#[test]
fn ranker_training_candidates_purpose_flag_parses_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "ranker-training-candidates",
        "--endpoint",
        "https://trace.example/internal",
        "--purpose",
        "nightly-ranker-candidates",
    ]);

    let TracesSubcommand::RankerTrainingCandidates { purpose, .. } = unwrap_traces_command(cli)
    else {
        panic!("expected traces ranker-training-candidates command");
    };

    assert_eq!(purpose.as_deref(), Some("nightly-ranker-candidates"));
}

#[test]
fn worker_ranker_training_candidates_parses_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "worker-ranker-training-candidates",
        "--endpoint",
        "https://trace.example/internal",
        "--purpose",
        "nightly-worker-candidates",
        "--bearer-token-env",
        "TRACE_COMMONS_EXPORT_WORKER_TOKEN",
    ]);

    let TracesSubcommand::WorkerRankerTrainingCandidates {
        purpose,
        bearer_token_env,
        ..
    } = unwrap_traces_command(cli)
    else {
        panic!("expected traces worker-ranker-training-candidates command");
    };

    assert_eq!(purpose.as_deref(), Some("nightly-worker-candidates"));
    assert_eq!(bearer_token_env, "TRACE_COMMONS_EXPORT_WORKER_TOKEN");
}

#[test]
fn ranker_training_pairs_purpose_flag_parses_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "ranker-training-pairs",
        "--endpoint",
        "https://trace.example/internal",
        "--purpose",
        "nightly-ranker-pairs",
    ]);

    let TracesSubcommand::RankerTrainingPairs { purpose, .. } = unwrap_traces_command(cli) else {
        panic!("expected traces ranker-training-pairs command");
    };

    assert_eq!(purpose.as_deref(), Some("nightly-ranker-pairs"));
}

#[test]
fn worker_ranker_training_pairs_parses_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "worker-ranker-training-pairs",
        "--endpoint",
        "https://trace.example/internal",
        "--purpose",
        "nightly-worker-pairs",
        "--bearer-token-env",
        "TRACE_COMMONS_EXPORT_WORKER_TOKEN",
    ]);

    let TracesSubcommand::WorkerRankerTrainingPairs {
        purpose,
        bearer_token_env,
        ..
    } = unwrap_traces_command(cli)
    else {
        panic!("expected traces worker-ranker-training-pairs command");
    };

    assert_eq!(purpose.as_deref(), Some("nightly-worker-pairs"));
    assert_eq!(bearer_token_env, "TRACE_COMMONS_EXPORT_WORKER_TOKEN");
}

#[test]
fn ranker_training_pairs_use_ingest_endpoint() {
    let url = trace_commons_api_url(
        "https://trace.example/internal/v1",
        "/v1/ranker/training-pairs",
        &[("privacy_risk", "low".to_string())],
    )
    .expect("url builds");

    assert_eq!(
        url,
        "https://trace.example/internal/v1/ranker/training-pairs?privacy_risk=low"
    );
}

#[test]
fn worker_ranker_training_candidates_use_dedicated_route() {
    let url = trace_commons_api_url(
        "https://trace.example/internal/v1",
        "/v1/workers/ranker/training-candidates",
        &[("purpose", "nightly-worker-candidates".to_string())],
    )
    .expect("url builds");

    assert_eq!(
        url,
        "https://trace.example/internal/v1/workers/ranker/training-candidates?purpose=nightly-worker-candidates"
    );
}

#[test]
fn worker_ranker_training_pairs_use_dedicated_route() {
    let url = trace_commons_api_url(
        "https://trace.example/internal/v1",
        "/v1/workers/ranker/training-pairs",
        &[("privacy_risk", "low".to_string())],
    )
    .expect("url builds");

    assert_eq!(
        url,
        "https://trace.example/internal/v1/workers/ranker/training-pairs?privacy_risk=low"
    );
}

#[test]
fn tenant_policy_get_parses_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "tenant-policy-get",
        "--endpoint",
        "https://trace.example/internal",
        "--json",
    ]);

    let TracesSubcommand::TenantPolicyGet { endpoint, json, .. } = unwrap_traces_command(cli)
    else {
        panic!("expected traces tenant-policy-get command");
    };

    assert_eq!(endpoint, "https://trace.example/internal");
    assert!(json);
}

#[test]
fn config_status_parses_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "config-status",
        "--endpoint",
        "https://trace.example/internal",
        "--bearer-token-env",
        "TRACE_COMMONS_ADMIN_TOKEN",
    ]);

    let TracesSubcommand::ConfigStatus {
        endpoint,
        bearer_token_env,
    } = unwrap_traces_command(cli)
    else {
        panic!("expected traces config-status command");
    };

    assert_eq!(endpoint, "https://trace.example/internal");
    assert_eq!(bearer_token_env, "TRACE_COMMONS_ADMIN_TOKEN");
}

#[test]
fn operational_summary_parses_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "operational-summary",
        "--endpoint",
        "https://trace.example/internal",
        "--bearer-token-env",
        "TRACE_COMMONS_ADMIN_TOKEN",
        "--json",
    ]);

    let TracesSubcommand::OperationalSummary {
        endpoint,
        bearer_token_env,
        json,
    } = unwrap_traces_command(cli)
    else {
        panic!("expected traces operational-summary command");
    };

    assert_eq!(endpoint, "https://trace.example/internal");
    assert_eq!(bearer_token_env, "TRACE_COMMONS_ADMIN_TOKEN");
    assert!(json);
}

#[test]
fn tenant_policy_set_parses_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "tenant-policy-set",
        "--endpoint",
        "https://trace.example/internal",
        "--policy-version",
        "2026-04-26",
        "--allowed-consent-scopes",
        "debugging-evaluation,ranking-training",
        "--allowed-uses",
        "debugging,ranking-model-training",
    ]);

    let TracesSubcommand::TenantPolicySet {
        policy_version,
        allowed_consent_scopes,
        allowed_uses,
        ..
    } = unwrap_traces_command(cli)
    else {
        panic!("expected traces tenant-policy-set command");
    };

    assert_eq!(policy_version, "2026-04-26");
    assert_eq!(
        allowed_consent_scopes,
        vec![
            TraceScopeArg::DebuggingEvaluation,
            TraceScopeArg::RankingTraining
        ]
    );
    assert_eq!(
        allowed_uses,
        vec![
            TraceAllowedUseArg::Debugging,
            TraceAllowedUseArg::RankingModelTraining
        ]
    );
}

#[test]
fn tenant_policy_uses_ingest_endpoint() {
    let url = trace_commons_api_url(
        "https://trace.example/internal/v1/traces",
        "/v1/admin/tenant-policy",
        &[],
    )
    .expect("url builds");

    assert_eq!(url, "https://trace.example/internal/v1/admin/tenant-policy");
}

#[test]
fn config_status_uses_ingest_endpoint() {
    let url = trace_commons_api_url(
        "https://trace.example/internal/v1/traces",
        "/v1/admin/config-status",
        &[],
    )
    .expect("url builds");

    assert_eq!(url, "https://trace.example/internal/v1/admin/config-status");
}

#[test]
fn operational_summary_uses_ingest_endpoint() {
    let url = trace_commons_api_url(
        "https://trace.example/internal/v1/traces",
        "/v1/admin/operational-summary",
        &[],
    )
    .expect("url builds");

    assert_eq!(
        url,
        "https://trace.example/internal/v1/admin/operational-summary"
    );
}

#[test]
fn operational_summary_lines_render_aggregate_counts_without_item_ids() {
    let value = serde_json::json!({
        "tenant_id": "tenant-a",
        "tenant_storage_ref": "tenant:tenant-a",
        "generated_at": "2026-04-28T10:00:00Z",
        "submissions": {
            "total": 2,
            "accepted": 1,
            "quarantined": 1,
            "rejected": 0,
            "revoked": 0,
            "expired": 0,
            "purged": 0,
            "by_status": {
                "accepted": 1,
                "quarantined": 1
            },
            "by_privacy_risk": {
                "low": 1,
                "high": 1
            },
            "example_submission_ids": [
                "11111111-1111-1111-1111-111111111111"
            ]
        },
        "review_sla": {
            "quarantined_total": 1,
            "fresh": 0,
            "due": 0,
            "overdue": 0,
            "urgent": 1,
            "assigned": 1,
            "expired_leases": 0,
            "oldest_age_hours": 36,
            "by_state": {
                "urgent": 1
            }
        },
        "exports": {
            "db_available": true,
            "manifest_count": 1,
            "active_manifest_count": 1,
            "invalidated_manifest_count": 0,
            "deleted_manifest_count": 0,
            "total_manifest_items": 1,
            "job_count": 1,
            "by_artifact_kind": {
                "replay_dataset": 1
            },
            "jobs_by_status": {
                "complete": 1
            }
        },
        "retention": {
            "db_available": true,
            "job_count": 1,
            "dry_run_count": 1,
            "selected_revoked_total": 0,
            "selected_expired_total": 1,
            "jobs_by_status": {
                "dry_run": 1
            }
        },
        "vectors": {
            "db_available": true,
            "entry_count": 1,
            "active_entries": 1,
            "invalidated_entries": 0,
            "deleted_entries": 0,
            "accepted_current_derived": 1,
            "accepted_current_derived_with_active_vector": 1,
            "active_coverage_percent": 100.0
        },
        "delayed_credit": {
            "event_count": 1,
            "points_positive": 0.25,
            "points_negative": 0.0,
            "points_total": 0.25,
            "last_event_at": "2026-04-28T10:05:00Z",
            "by_event_type": {
                "reviewer_bonus": 1
            }
        },
        "promotion_gates": {
            "ready": false,
            "blocking_count": 1,
            "warning_count": 1,
            "blocking_gates": [
                "urgent_reviews=1"
            ],
            "warning_gates": [
                "tenant_rollout_gates=3"
            ],
            "db_mirror_configured": true,
            "require_db_mirror_writes": true,
            "require_db_reconciliation_clean": true,
            "require_derived_export_object_refs": true,
            "require_export_guardrails": true,
            "object_primary_submit_review": false,
            "object_primary_replay_export": true,
            "object_primary_derived_exports": false,
            "tenant_rollout_gate_count": 3,
            "tenant_rollout_gate_counts": {
                "db_contributor_reads": 2,
                "object_primary": 1
            },
            "open_review_count": 1,
            "urgent_review_count": 1,
            "failed_export_job_count": 0,
            "failed_retention_job_count": 0,
            "vector_missing_count": 0
        }
    });

    let lines = trace_commons_operational_summary_lines(&value);
    let rendered = lines.join("\n");

    assert!(rendered.contains("tenant: tenant-a"));
    assert!(rendered.contains("submissions:"));
    assert!(rendered.contains("totals: total=2 accepted=1 quarantined=1"));
    assert!(rendered.contains("review sla:"));
    assert!(rendered.contains("urgent=1"));
    assert!(rendered.contains("export:"));
    assert!(rendered.contains("by_artifact_kind: replay_dataset=1"));
    assert!(rendered.contains("retention:"));
    assert!(rendered.contains("vectors:"));
    assert!(rendered.contains("active_coverage_percent=100.0"));
    assert!(rendered.contains("delayed credit:"));
    assert!(rendered.contains("by_event_type: reviewer_bonus=1"));
    assert!(rendered.contains("promotion gates:"));
    assert!(rendered.contains("ready=false"));
    assert!(rendered.contains("blocking_count=1"));
    assert!(rendered.contains("require_db_reconciliation_clean=true"));
    assert!(rendered.contains("object_primary_replay_export=true"));
    assert!(rendered.contains("tenant_rollout_gate_count=3"));
    assert!(rendered.contains("blocking_gates: urgent_reviews=1"));
    assert!(rendered.contains("warning_gates: tenant_rollout_gates=3"));
    assert!(rendered.contains("tenant_rollout_gate_counts: db_contributor_reads=2"));
    assert!(!rendered.contains("11111111-1111-1111-1111-111111111111"));
}

#[test]
fn tenant_policy_body_uses_server_enum_names() {
    let body = trace_commons_tenant_policy_body(
        "2026-04-26".to_string(),
        vec![
            TraceScopeArg::DebuggingEvaluation,
            TraceScopeArg::BenchmarkOnly,
        ],
        vec![
            TraceAllowedUseArg::Debugging,
            TraceAllowedUseArg::AggregateAnalytics,
            TraceAllowedUseArg::RankingModelTraining,
        ],
    );

    assert_eq!(body["policy_version"], "2026-04-26");
    assert_eq!(
        body["allowed_consent_scopes"],
        serde_json::json!(["debugging_evaluation", "benchmark_only"])
    );
    assert_eq!(
        body["allowed_uses"],
        serde_json::json!(["debugging", "aggregate_analytics", "ranking_model_training"])
    );
}

#[test]
fn tenant_access_grants_list_parses_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "tenant-access-grants-list",
        "--endpoint",
        "https://trace.example/internal",
        "--limit",
        "25",
        "--status",
        "active",
        "--role",
        "contributor",
        "--principal-ref",
        "principal_sha256:abc",
        "--bearer-token-env",
        "TRACE_COMMONS_ADMIN_TOKEN",
        "--json",
    ]);

    let TracesSubcommand::TenantAccessGrantsList {
        endpoint,
        limit,
        status,
        role,
        principal_ref,
        bearer_token_env,
        json,
    } = unwrap_traces_command(cli)
    else {
        panic!("expected traces tenant-access-grants-list command");
    };

    assert_eq!(endpoint, "https://trace.example/internal");
    assert_eq!(limit, Some(25));
    assert_eq!(status, Some(TraceTenantAccessGrantStatusArg::Active));
    assert_eq!(role, Some(TraceTenantAccessGrantRoleArg::Contributor));
    assert_eq!(principal_ref.as_deref(), Some("principal_sha256:abc"));
    assert_eq!(bearer_token_env, "TRACE_COMMONS_ADMIN_TOKEN");
    assert!(json);
}

#[test]
fn tenant_principal_ref_parses_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "tenant-principal-ref",
        "--signed-tenant-id",
        "tenant-a",
        "--signed-actor-ref",
        "actor-123",
        "--json",
    ]);

    let TracesSubcommand::TenantPrincipalRef {
        signed_tenant_id,
        signed_actor_ref,
        json,
        ..
    } = unwrap_traces_command(cli)
    else {
        panic!("expected traces tenant-principal-ref command");
    };

    assert_eq!(signed_tenant_id.as_deref(), Some("tenant-a"));
    assert_eq!(signed_actor_ref.as_deref(), Some("actor-123"));
    assert!(json);
}

#[test]
fn tenant_principal_ref_uses_ingest_principal_hash_shape() {
    let signed = trace_commons_signed_claim_principal_ref("tenant-a", "actor-123");
    let expected = trace_commons_principal_storage_ref("signed:tenant-a:actor-123");

    assert_eq!(signed, expected);
    assert!(signed.starts_with("principal_sha256:"));
}

#[test]
fn tenant_access_grant_create_parses_through_cli() {
    let grant_id = "018f2b7b-0c11-72fd-95c4-1f9f98feac01";
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "tenant-access-grant-create",
        "--endpoint",
        "https://trace.example/internal",
        "--principal-ref",
        "principal_sha256:abc",
        "--role",
        "process-eval-worker",
        "--grant-id",
        grant_id,
        "--allowed-consent-scopes",
        "debugging-evaluation,ranking-training",
        "--allowed-uses",
        "debugging,ranking-model-training",
        "--issuer",
        "https://issuer.near.com",
        "--audience",
        "trace-commons",
        "--subject",
        "tenant-a-agent",
        "--issued-at",
        "2026-04-27T00:00:00Z",
        "--expires-at",
        "2026-04-28T00:00:00Z",
        "--reason",
        "hosted tenant verified",
        "--metadata",
        "hosted_surface=near.com",
        "--bearer-token-env",
        "TRACE_COMMONS_ADMIN_TOKEN",
    ]);

    let TracesSubcommand::TenantAccessGrantCreate {
        principal_ref,
        role,
        grant_id: parsed_grant_id,
        allowed_consent_scopes,
        allowed_uses,
        issuer,
        audience,
        subject,
        issued_at,
        expires_at,
        reason,
        metadata,
        bearer_token_env,
        ..
    } = unwrap_traces_command(cli)
    else {
        panic!("expected traces tenant-access-grant-create command");
    };

    assert_eq!(principal_ref, "principal_sha256:abc");
    assert_eq!(role, TraceTenantAccessGrantRoleArg::ProcessEvalWorker);
    assert_eq!(parsed_grant_id, Some(Uuid::parse_str(grant_id).unwrap()));
    assert_eq!(
        allowed_consent_scopes,
        vec![
            TraceScopeArg::DebuggingEvaluation,
            TraceScopeArg::RankingTraining
        ]
    );
    assert_eq!(
        allowed_uses,
        vec![
            TraceAllowedUseArg::Debugging,
            TraceAllowedUseArg::RankingModelTraining
        ]
    );
    assert_eq!(issuer.as_deref(), Some("https://issuer.near.com"));
    assert_eq!(audience.as_deref(), Some("trace-commons"));
    assert_eq!(subject.as_deref(), Some("tenant-a-agent"));
    assert_eq!(issued_at.as_deref(), Some("2026-04-27T00:00:00Z"));
    assert_eq!(expires_at.as_deref(), Some("2026-04-28T00:00:00Z"));
    assert_eq!(reason, "hosted tenant verified");
    assert_eq!(metadata, vec!["hosted_surface=near.com"]);
    assert_eq!(bearer_token_env, "TRACE_COMMONS_ADMIN_TOKEN");
}

#[test]
fn tenant_access_grant_revoke_parses_through_cli() {
    let grant_id = "018f2b7b-0c11-72fd-95c4-1f9f98feac01";
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "tenant-access-grant-revoke",
        "--endpoint",
        "https://trace.example/internal",
        grant_id,
        "--reason",
        "tenant deprovisioned",
        "--bearer-token-env",
        "TRACE_COMMONS_ADMIN_TOKEN",
        "--json",
    ]);

    let TracesSubcommand::TenantAccessGrantRevoke {
        grant_id: parsed_grant_id,
        reason,
        bearer_token_env,
        json,
        ..
    } = unwrap_traces_command(cli)
    else {
        panic!("expected traces tenant-access-grant-revoke command");
    };

    assert_eq!(parsed_grant_id, Uuid::parse_str(grant_id).unwrap());
    assert_eq!(reason, "tenant deprovisioned");
    assert_eq!(bearer_token_env, "TRACE_COMMONS_ADMIN_TOKEN");
    assert!(json);
}

#[test]
fn tenant_access_grants_use_ingest_endpoint() {
    let url = trace_commons_api_url(
        "https://trace.example/internal/v1/traces",
        "/v1/admin/tenant-access-grants",
        &[
            ("status", "active".to_string()),
            ("role", "export_worker".to_string()),
        ],
    )
    .expect("url builds");

    assert_eq!(
        url,
        "https://trace.example/internal/v1/admin/tenant-access-grants?status=active&role=export_worker"
    );
}

#[test]
fn tenant_access_grant_revoke_uses_ingest_endpoint() {
    let grant_id = Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").unwrap();
    let url = trace_commons_api_url(
        "https://trace.example/internal/v1/traces",
        &format!("/v1/admin/tenant-access-grants/{grant_id}/revoke"),
        &[],
    )
    .expect("url builds");

    assert_eq!(
        url,
        "https://trace.example/internal/v1/admin/tenant-access-grants/018f2b7b-0c11-72fd-95c4-1f9f98feac01/revoke"
    );
}

#[test]
fn tenant_access_grant_body_uses_server_enum_names_and_metadata() {
    let options = TraceCommonsTenantAccessGrantCreateOptions {
        endpoint: "https://trace.example/internal",
        bearer_token_env: "TRACE_COMMONS_ADMIN_TOKEN",
        principal_ref: "principal_sha256:abc".to_string(),
        role: TraceTenantAccessGrantRoleArg::ExportWorker,
        grant_id: Some(Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").unwrap()),
        allowed_consent_scopes: vec![
            TraceScopeArg::DebuggingEvaluation,
            TraceScopeArg::BenchmarkOnly,
        ],
        allowed_uses: vec![
            TraceAllowedUseArg::Debugging,
            TraceAllowedUseArg::AggregateAnalytics,
            TraceAllowedUseArg::RankingModelTraining,
        ],
        issuer: Some(" https://issuer.near.com ".to_string()),
        audience: Some(" trace-commons ".to_string()),
        subject: Some(" tenant-a-agent ".to_string()),
        issued_at: Some("2026-04-27T00:00:00Z".to_string()),
        expires_at: Some("2026-04-28T00:00:00Z".to_string()),
        reason: " hosted tenant verified ".to_string(),
        metadata: vec![
            "hosted_surface=near.com".to_string(),
            "region=iad".to_string(),
        ],
        json: false,
    };

    let body = trace_commons_tenant_access_grant_body(&options).expect("body builds");
    assert_eq!(body["principal_ref"], "principal_sha256:abc");
    assert_eq!(body["role"], "export_worker");
    assert_eq!(
        body["allowed_consent_scopes"],
        serde_json::json!(["debugging_evaluation", "benchmark_only"])
    );
    assert_eq!(
        body["allowed_uses"],
        serde_json::json!(["debugging", "aggregate_analytics", "ranking_model_training"])
    );
    assert_eq!(body["issuer"], "https://issuer.near.com");
    assert_eq!(body["audience"], "trace-commons");
    assert_eq!(body["subject"], "tenant-a-agent");
    assert_eq!(body["reason"], "hosted tenant verified");
    assert_eq!(body["metadata"]["hosted_surface"], "near.com");
    assert_eq!(body["metadata"]["region"], "iad");
}

#[test]
fn worker_benchmark_convert_uses_dedicated_route() {
    let url = trace_commons_api_url(
        "https://trace.example/internal/v1/traces",
        "/v1/workers/benchmark-convert",
        &[],
    )
    .expect("url builds");

    assert_eq!(
        url,
        "https://trace.example/internal/v1/workers/benchmark-convert"
    );
}

#[test]
fn worker_replay_dataset_export_parses_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "worker-replay-dataset-export",
        "--endpoint",
        "https://trace.example/internal",
        "--purpose",
        "nightly-worker-replay",
        "--bearer-token-env",
        "TRACE_COMMONS_EXPORT_WORKER_TOKEN",
    ]);

    let TracesSubcommand::WorkerReplayDatasetExport {
        purpose,
        bearer_token_env,
        ..
    } = unwrap_traces_command(cli)
    else {
        panic!("expected traces worker-replay-dataset-export command");
    };

    assert_eq!(purpose.as_deref(), Some("nightly-worker-replay"));
    assert_eq!(bearer_token_env, "TRACE_COMMONS_EXPORT_WORKER_TOKEN");
}

#[test]
fn worker_replay_dataset_export_uses_dedicated_route() {
    let url = trace_commons_api_url(
        "https://trace.example/internal/v1/traces",
        "/v1/workers/replay-export",
        &[("purpose", "nightly-worker-replay".to_string())],
    )
    .expect("url builds");

    assert_eq!(
        url,
        "https://trace.example/internal/v1/workers/replay-export?purpose=nightly-worker-replay"
    );
}

#[test]
fn benchmark_lifecycle_update_uses_conversion_route() {
    let conversion_id =
        Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("uuid parses");
    let url = trace_commons_api_url(
        "https://trace.example/internal/v1/traces",
        &format!("/v1/benchmarks/{conversion_id}/lifecycle"),
        &[],
    )
    .expect("url builds");

    assert_eq!(
        url,
        "https://trace.example/internal/v1/benchmarks/018f2b7b-0c11-72fd-95c4-1f9f98feac01/lifecycle"
    );
}

#[test]
fn benchmark_lifecycle_update_parses_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "benchmark-lifecycle-update",
        "--endpoint",
        "https://trace.example/internal",
        "--conversion-id",
        "018f2b7b-0c11-72fd-95c4-1f9f98feac01",
        "--registry-status",
        "published",
        "--registry-ref",
        "benchmark:trace-018",
        "--published-at",
        "2026-04-26T12:00:00Z",
        "--evaluation-status",
        "passed",
        "--evaluator-ref",
        "eval:nightly-42",
        "--evaluated-at",
        "2026-04-26T12:05:00Z",
        "--score",
        "0.97",
        "--pass-count",
        "7",
        "--fail-count",
        "0",
        "--reason",
        "published after evaluator pass",
        "--bearer-token-env",
        "TRACE_COMMONS_BENCHMARK_WORKER_TOKEN",
    ]);

    let TracesSubcommand::BenchmarkLifecycleUpdate {
        conversion_id,
        registry_status,
        registry_ref,
        published_at,
        evaluation_status,
        evaluator_ref,
        evaluated_at,
        score,
        pass_count,
        fail_count,
        reason,
        bearer_token_env,
        ..
    } = unwrap_traces_command(cli)
    else {
        panic!("expected traces benchmark-lifecycle-update command");
    };

    assert_eq!(
        conversion_id,
        Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("uuid parses")
    );
    assert_eq!(
        registry_status,
        Some(TraceBenchmarkRegistryStatusArg::Published)
    );
    assert_eq!(registry_ref.as_deref(), Some("benchmark:trace-018"));
    assert_eq!(published_at.as_deref(), Some("2026-04-26T12:00:00Z"));
    assert_eq!(
        evaluation_status,
        Some(TraceBenchmarkEvaluationStatusArg::Passed)
    );
    assert_eq!(evaluator_ref.as_deref(), Some("eval:nightly-42"));
    assert_eq!(evaluated_at.as_deref(), Some("2026-04-26T12:05:00Z"));
    assert_eq!(score, Some(0.97));
    assert_eq!(pass_count, Some(7));
    assert_eq!(fail_count, Some(0));
    assert_eq!(reason.as_deref(), Some("published after evaluator pass"));
    assert_eq!(bearer_token_env, "TRACE_COMMONS_BENCHMARK_WORKER_TOKEN");
}

#[test]
fn benchmark_lifecycle_body_uses_nested_server_fields() {
    let body = trace_commons_benchmark_lifecycle_body(
        Some(TraceBenchmarkRegistryStatusArg::Published),
        Some("benchmark:trace-018".to_string()),
        Some("2026-04-26T12:00:00Z".to_string()),
        Some(TraceBenchmarkEvaluationStatusArg::Passed),
        Some("eval:nightly-42".to_string()),
        Some("2026-04-26T12:05:00Z".to_string()),
        Some(0.5),
        Some(7),
        Some(0),
        Some("published after evaluator pass".to_string()),
    )
    .expect("body builds");

    assert_eq!(
        body,
        serde_json::json!({
            "registry": {
                "status": "published",
                "registry_ref": "benchmark:trace-018",
                "published_at": "2026-04-26T12:00:00Z"
            },
            "evaluation": {
                "status": "passed",
                "evaluator_ref": "eval:nightly-42",
                "evaluated_at": "2026-04-26T12:05:00Z",
                "score": 0.5,
                "pass_count": 7,
                "fail_count": 0
            },
            "reason": "published after evaluator pass"
        })
    );
}

#[test]
fn benchmark_lifecycle_body_rejects_empty_and_invalid_score() {
    let empty = trace_commons_benchmark_lifecycle_body(
        None, None, None, None, None, None, None, None, None, None,
    );
    assert!(empty.is_err());

    let invalid_score = trace_commons_benchmark_lifecycle_body(
        None,
        None,
        None,
        Some(TraceBenchmarkEvaluationStatusArg::Passed),
        None,
        None,
        Some(1.5),
        None,
        None,
        None,
    );
    assert!(invalid_score.is_err());
}

#[test]
fn worker_commands_parse_through_cli() {
    let benchmark = parse_cli([
        "ironclaw-reborn",
        "traces",
        "worker-benchmark-convert",
        "--endpoint",
        "https://trace.example/internal",
        "--purpose",
        "nightly-worker-benchmark",
        "--status",
        "accepted",
    ]);
    let TracesSubcommand::WorkerBenchmarkConvert {
        purpose, status, ..
    } = unwrap_traces_command(benchmark)
    else {
        panic!("expected traces worker-benchmark-convert command");
    };
    assert_eq!(purpose, "nightly-worker-benchmark");
    assert_eq!(status, Some(TraceCorpusStatusArg::Accepted));

    let retention = parse_cli([
        "ironclaw-reborn",
        "traces",
        "worker-retention-maintenance",
        "--endpoint",
        "https://trace.example/internal",
        "--purpose",
        "retention-worker",
        "--dry-run",
    ]);
    let TracesSubcommand::WorkerRetentionMaintenance {
        purpose, dry_run, ..
    } = unwrap_traces_command(retention)
    else {
        panic!("expected traces worker-retention-maintenance command");
    };
    assert_eq!(purpose.as_deref(), Some("retention-worker"));
    assert!(dry_run);

    let vector = parse_cli([
        "ironclaw-reborn",
        "traces",
        "worker-vector-index",
        "--endpoint",
        "https://trace.example/internal",
        "--purpose",
        "vector-worker",
    ]);
    let TracesSubcommand::WorkerVectorIndex { purpose, .. } = unwrap_traces_command(vector) else {
        panic!("expected traces worker-vector-index command");
    };
    assert_eq!(purpose.as_deref(), Some("vector-worker"));

    let utility_submission_id =
        Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("valid uuid");
    let utility = parse_cli([
        "ironclaw-reborn",
        "traces",
        "worker-utility-credit",
        "--endpoint",
        "https://trace.example/internal",
        "--event-type",
        "ranking-utility",
        "--credit-points-delta",
        "1.25",
        "--reason",
        "ranking eval utility",
        "--external-ref",
        "ranker:nightly-42",
        "018f2b7b-0c11-72fd-95c4-1f9f98feac01",
    ]);
    let TracesSubcommand::WorkerUtilityCredit {
        event_type,
        credit_points_delta,
        external_ref,
        submission_ids,
        ..
    } = unwrap_traces_command(utility)
    else {
        panic!("expected traces worker-utility-credit command");
    };
    assert_eq!(event_type, TraceUtilityCreditEventTypeArg::RankingUtility);
    assert_eq!(credit_points_delta, 1.25);
    assert_eq!(external_ref, "ranker:nightly-42");
    assert_eq!(submission_ids, vec![utility_submission_id]);
}

#[test]
fn worker_utility_credit_rejects_reviewer_and_abuse_events() {
    for event_type in ["reviewer-bonus", "abuse-penalty"] {
        let parsed = parse_cli_result([
            "ironclaw-reborn",
            "traces",
            "worker-utility-credit",
            "--endpoint",
            "https://trace.example/internal",
            "--event-type",
            event_type,
            "--credit-points-delta",
            "1.0",
            "--reason",
            "not a utility event",
            "--external-ref",
            "job:bad",
            "018f2b7b-0c11-72fd-95c4-1f9f98feac01",
        ]);
        assert!(parsed.is_err(), "{event_type} should not parse");
    }
}

#[test]
fn worker_utility_credit_body_uses_narrow_fields() {
    let first = Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("valid uuid");
    let second = Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac02").expect("valid uuid");
    let body = trace_commons_worker_utility_credit_body(
        TraceUtilityCreditEventTypeArg::TrainingUtility,
        2.0,
        "offline training utility".to_string(),
        "training:batch-7".to_string(),
        vec![first, second],
    );

    assert_eq!(body["event_type"], "training_utility");
    assert_eq!(body["credit_points_delta"], 2.0);
    assert_eq!(body["reason"], "offline training utility");
    assert_eq!(body["external_ref"], "training:batch-7");
    assert_eq!(
        body["submission_ids"],
        serde_json::json!([
            "018f2b7b-0c11-72fd-95c4-1f9f98feac01",
            "018f2b7b-0c11-72fd-95c4-1f9f98feac02"
        ])
    );
    assert!(body.get("reviewer_bonus").is_none());
    assert!(body.get("abuse_penalty").is_none());
}

#[test]
fn worker_utility_credit_uses_dedicated_route() {
    let url = trace_commons_api_url(
        "https://trace.example/internal/v1/traces",
        "/v1/workers/utility-credit",
        &[],
    )
    .expect("url builds");

    assert_eq!(
        url,
        "https://trace.example/internal/v1/workers/utility-credit"
    );
}

#[test]
fn process_evaluation_submit_parses_through_cli() {
    let submission_id =
        Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("valid uuid");
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "process-evaluation-submit",
        "--endpoint",
        "https://trace.example/internal",
        "--submission-id",
        "018f2b7b-0c11-72fd-95c4-1f9f98feac01",
        "--reason",
        "nightly evaluator pass",
        "--evaluator-name",
        "process-quality",
        "--evaluator-version",
        "2026-04-26.1",
        "--label",
        "proper_verification",
        "--label",
        "safe_tool_ordering",
        "--tool-selection",
        "pass",
        "--tool-argument-quality",
        "partial",
        "--tool-ordering",
        "not_applicable",
        "--verification",
        "fail",
        "--side-effect-safety",
        "unknown",
        "--overall-score",
        "0.75",
        "--utility-credit-points-delta",
        "2.5",
        "--utility-external-ref",
        "process-eval:nightly:42",
        "--bearer-token-env",
        "TRACE_COMMONS_PROCESS_EVALUATION_WORKER_TOKEN",
        "--json",
    ]);

    let TracesSubcommand::ProcessEvaluationSubmit(args) = unwrap_traces_command(cli) else {
        panic!("expected traces process-evaluation-submit command");
    };

    assert_eq!(args.endpoint, "https://trace.example/internal");
    assert_eq!(args.submission_id, submission_id);
    assert_eq!(args.reason, "nightly evaluator pass");
    assert_eq!(args.evaluator_name.as_deref(), Some("process-quality"));
    assert_eq!(args.evaluator_version, "2026-04-26.1");
    assert_eq!(
        args.labels,
        vec!["proper_verification", "safe_tool_ordering"]
    );
    assert_eq!(
        args.tool_selection,
        Some(TraceProcessEvaluationRatingArg::Pass)
    );
    assert_eq!(
        args.tool_argument_quality,
        Some(TraceProcessEvaluationRatingArg::Partial)
    );
    assert_eq!(
        args.tool_ordering,
        Some(TraceProcessEvaluationRatingArg::NotApplicable)
    );
    assert_eq!(
        args.verification,
        Some(TraceProcessEvaluationRatingArg::Fail)
    );
    assert_eq!(
        args.side_effect_safety,
        Some(TraceProcessEvaluationRatingArg::Unknown)
    );
    assert_eq!(args.overall_score, Some(0.75));
    assert_eq!(args.utility_credit_points_delta, Some(2.5));
    assert_eq!(
        args.utility_external_ref.as_deref(),
        Some("process-eval:nightly:42")
    );
    assert_eq!(
        args.bearer_token_env,
        "TRACE_COMMONS_PROCESS_EVALUATION_WORKER_TOKEN"
    );
    assert!(args.json);
}

#[test]
fn process_evaluation_body_uses_expected_shape() {
    let submission_id =
        Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("valid uuid");
    let body = trace_commons_process_evaluation_body(
        submission_id,
        " nightly evaluator pass ".to_string(),
        Some(" process-quality ".to_string()),
        " 2026-04-26.1 ".to_string(),
        vec![
            "proper_verification".to_string(),
            " ".to_string(),
            "safe_tool_ordering".to_string(),
        ],
        Some(TraceProcessEvaluationRatingArg::Pass),
        Some(TraceProcessEvaluationRatingArg::Partial),
        Some(TraceProcessEvaluationRatingArg::NotApplicable),
        Some(TraceProcessEvaluationRatingArg::Fail),
        Some(TraceProcessEvaluationRatingArg::Unknown),
        Some(0.75),
        Some(2.5),
        Some(" process-eval:nightly:42 ".to_string()),
    )
    .expect("body builds");

    assert_eq!(
        body,
        serde_json::json!({
            "submission_id": "018f2b7b-0c11-72fd-95c4-1f9f98feac01",
            "process_evaluation": {
                "evaluator_name": "process-quality",
                "evaluator_version": "2026-04-26.1",
                "labels": ["proper_verification", "safe_tool_ordering"],
                "tool_selection": "pass",
                "tool_argument_quality": "partial",
                "tool_ordering": "not_applicable",
                "verification": "fail",
                "side_effect_safety": "unknown",
                "overall_score": 0.75
            },
            "utility_credit_points_delta": 2.5,
            "utility_external_ref": "process-eval:nightly:42",
            "reason": "nightly evaluator pass"
        })
    );
}

#[test]
fn process_evaluation_body_omits_utility_credit_when_delta_absent() {
    let submission_id =
        Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("valid uuid");
    let body = trace_commons_process_evaluation_body(
        submission_id,
        "operator reason".to_string(),
        None,
        "2026-04-26.1".to_string(),
        Vec::new(),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .expect("body builds");

    assert!(body.get("utility_credit_points_delta").is_none());
    assert!(body.get("utility_external_ref").is_none());
}

#[test]
fn process_evaluation_body_rejects_invalid_inputs() {
    let submission_id =
        Uuid::parse_str("018f2b7b-0c11-72fd-95c4-1f9f98feac01").expect("valid uuid");

    let empty_reason = trace_commons_process_evaluation_body(
        submission_id,
        " ".to_string(),
        None,
        "2026-04-26.1".to_string(),
        Vec::new(),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );
    assert!(empty_reason.is_err());

    let empty_version = trace_commons_process_evaluation_body(
        submission_id,
        "operator reason".to_string(),
        None,
        " ".to_string(),
        Vec::new(),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );
    assert!(empty_version.is_err());

    let invalid_score = trace_commons_process_evaluation_body(
        submission_id,
        "operator reason".to_string(),
        None,
        "2026-04-26.1".to_string(),
        Vec::new(),
        None,
        None,
        None,
        None,
        None,
        Some(1.25),
        None,
        None,
    );
    assert!(invalid_score.is_err());

    let non_finite_delta = trace_commons_process_evaluation_body(
        submission_id,
        "operator reason".to_string(),
        None,
        "2026-04-26.1".to_string(),
        Vec::new(),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(f32::NAN),
        Some("process-eval:nightly:42".to_string()),
    );
    assert!(non_finite_delta.is_err());

    let too_large_delta = trace_commons_process_evaluation_body(
        submission_id,
        "operator reason".to_string(),
        None,
        "2026-04-26.1".to_string(),
        Vec::new(),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(100.01),
        Some("process-eval:nightly:42".to_string()),
    );
    assert!(too_large_delta.is_err());

    let empty_external_ref = trace_commons_process_evaluation_body(
        submission_id,
        "operator reason".to_string(),
        None,
        "2026-04-26.1".to_string(),
        Vec::new(),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(2.5),
        Some(" ".to_string()),
    );
    assert!(empty_external_ref.is_err());

    let dangling_external_ref = trace_commons_process_evaluation_body(
        submission_id,
        "operator reason".to_string(),
        None,
        "2026-04-26.1".to_string(),
        Vec::new(),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some("process-eval:nightly:42".to_string()),
    );
    assert!(dangling_external_ref.is_err());
}

#[test]
fn process_evaluation_submit_uses_dedicated_route() {
    let url = trace_commons_api_url(
        "https://trace.example/internal/v1/traces",
        "/v1/workers/process-evaluation",
        &[],
    )
    .expect("url builds");

    assert_eq!(
        url,
        "https://trace.example/internal/v1/workers/process-evaluation"
    );
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

#[test]
fn worker_retention_maintenance_body_uses_narrow_fields() {
    let body = trace_commons_retention_maintenance_body(
        Some("retention-cutover".to_string()),
        true,
        false,
        Some(24),
        Some("2026-04-26T00:00:00Z".to_string()),
    );

    assert_eq!(body["purpose"], "retention-cutover");
    assert_eq!(body["dry_run"], true);
    assert_eq!(body["prune_export_cache"], false);
    assert_eq!(body["max_export_age_hours"], 24);
    assert_eq!(body["purge_expired_before"], "2026-04-26T00:00:00Z");
    assert!(body.get("backfill_db_mirror").is_none());
    assert!(body.get("index_vectors").is_none());
    assert!(body.get("reconcile_db_mirror").is_none());
}

#[test]
fn maintenance_summary_lines_include_benchmark_artifact_invalidation() {
    let value = serde_json::json!({
        "audit_event_id": "audit-123",
        "purpose": "source-invalidation",
        "dry_run": false,
        "benchmark_artifacts_invalidated": 3
    });

    let lines = trace_commons_maintenance_summary_lines(&value);

    assert!(lines.contains(&"  benchmark artifacts invalidated: 3".to_string()));
}

#[test]
fn retention_maintenance_summary_lines_include_benchmark_artifact_invalidation() {
    let value = serde_json::json!({
        "audit_event_id": "audit-123",
        "purpose": "retention-sweep",
        "dry_run": false,
        "benchmark_artifacts_invalidated": 2
    });

    let lines = trace_commons_retention_maintenance_summary_lines(&value);

    assert!(lines.contains(&"  benchmark artifacts invalidated: 2".to_string()));
}

#[test]
fn worker_vector_index_body_uses_narrow_fields() {
    let body = trace_commons_vector_index_body(Some("vector-refresh".to_string()), true);

    assert_eq!(body["purpose"], "vector-refresh");
    assert_eq!(body["dry_run"], true);
    assert!(body.get("backfill_db_mirror").is_none());
    assert!(body.get("reconcile_db_mirror").is_none());
}

#[test]
fn replay_export_manifests_use_ingest_endpoint() {
    let url = trace_commons_api_url(
        "https://trace.example/internal/v1/traces",
        "/v1/datasets/replay/manifests",
        &[],
    )
    .expect("url builds");

    assert_eq!(
        url,
        "https://trace.example/internal/v1/datasets/replay/manifests"
    );
}

#[test]
fn retention_jobs_list_parses_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "retention-jobs-list",
        "--endpoint",
        "https://trace.example/internal",
        "--limit",
        "25",
        "--status",
        "dry-run",
        "--bearer-token-env",
        "TRACE_COMMONS_ADMIN_TOKEN",
    ]);

    let TracesSubcommand::RetentionJobsList {
        endpoint,
        limit,
        status,
        bearer_token_env,
        json,
    } = unwrap_traces_command(cli)
    else {
        panic!("expected traces retention-jobs-list command");
    };

    assert_eq!(endpoint, "https://trace.example/internal");
    assert_eq!(limit, Some(25));
    assert_eq!(status, Some(TraceRetentionJobStatusArg::DryRun));
    assert_eq!(bearer_token_env, "TRACE_COMMONS_ADMIN_TOKEN");
    assert!(!json);
}

#[test]
fn retention_job_items_use_ingest_endpoint() {
    let job_id =
        Uuid::parse_str("11111111-1111-1111-1111-111111111111").expect("static uuid parses");
    let url = trace_commons_api_url(
        "https://trace.example/internal/v1/traces",
        &format!("/v1/admin/retention/jobs/{job_id}/items"),
        &[
            ("action", "purge".to_string()),
            ("status", "done".to_string()),
        ],
    )
    .expect("url builds");

    assert_eq!(
        url,
        "https://trace.example/internal/v1/admin/retention/jobs/11111111-1111-1111-1111-111111111111/items?action=purge&status=done"
    );
}

#[test]
fn export_access_grants_list_parses_through_cli() {
    let cli = parse_cli([
        "ironclaw-reborn",
        "traces",
        "export-access-grants-list",
        "--endpoint",
        "https://trace.example/internal",
        "--limit",
        "25",
        "--status",
        "active",
        "--dataset-kind",
        "replay-dataset",
        "--bearer-token-env",
        "TRACE_COMMONS_ADMIN_TOKEN",
    ]);

    let TracesSubcommand::ExportAccessGrantsList {
        endpoint,
        limit,
        status,
        dataset_kind,
        bearer_token_env,
        json,
    } = unwrap_traces_command(cli)
    else {
        panic!("expected traces export-access-grants-list command");
    };

    assert_eq!(endpoint, "https://trace.example/internal");
    assert_eq!(limit, Some(25));
    assert_eq!(status, Some(TraceExportAccessGrantStatusArg::Active));
    assert_eq!(dataset_kind.as_deref(), Some("replay-dataset"));
    assert_eq!(bearer_token_env, "TRACE_COMMONS_ADMIN_TOKEN");
    assert!(!json);
}

#[test]
fn export_jobs_list_uses_ingest_endpoint() {
    let url = trace_commons_api_url(
        "https://trace.example/internal/v1/traces",
        "/v1/admin/export/jobs",
        &[
            ("status", "complete".to_string()),
            ("dataset_kind", "ranker_training_pairs".to_string()),
        ],
    )
    .expect("url builds");

    assert_eq!(
        url,
        "https://trace.example/internal/v1/admin/export/jobs?status=complete&dataset_kind=ranker_training_pairs"
    );
}

#[test]
fn list_traces_purpose_query_uses_ingest_endpoint() {
    let url = trace_commons_api_url(
        "https://trace.example/internal/v1/traces",
        "/v1/traces",
        &[
            ("purpose", "ranker_training_pairs_export".to_string()),
            ("limit", "20".to_string()),
        ],
    )
    .expect("url builds");

    assert_eq!(
        url,
        "https://trace.example/internal/v1/traces?purpose=ranker_training_pairs_export&limit=20"
    );
}

#[test]
fn maintenance_reconciliation_lines_summarize_counts_without_ids() {
    let value: serde_json::Value = serde_json::from_str(
        r#"{
        "db_reconciliation": {
            "file_submission_count": 3,
            "db_submission_count": 2,
            "missing_submission_ids_in_db": ["missing-a", "missing-b"],
            "missing_submission_ids_in_files": [],
            "status_mismatches": ["status-mismatch"],
            "file_derived_count": 4,
            "db_derived_count": 3,
            "missing_derived_submission_ids_in_db": ["derived-missing-db"],
            "missing_derived_submission_ids_in_files": ["derived-missing-files"],
            "derived_status_mismatches": ["derived-status"],
            "derived_hash_mismatches": ["derived-hash"],
            "db_object_ref_count": 2,
            "accepted_without_active_envelope_object_ref": ["missing-object-ref"],
            "unreadable_active_envelope_object_refs": ["unreadable-object-ref"],
            "hash_mismatched_active_envelope_object_refs": ["hash-mismatch"],
            "file_credit_event_count": 5,
            "db_credit_event_count": 4,
            "file_audit_event_count": 6,
            "db_audit_event_count": 6,
            "db_retention_job_count": 2,
            "db_retention_job_item_count": 4,
            "file_replay_export_manifest_count": 1,
            "db_export_manifest_count": 2,
            "db_replay_export_manifest_count": 1,
            "db_benchmark_export_manifest_count": 0,
            "db_ranker_export_manifest_count": 1,
            "db_other_export_manifest_count": 0,
            "db_export_manifest_item_count": 3,
            "file_revocation_tombstone_count": 1,
            "db_tombstone_count": 1,
            "contributor_credit_reader_parity_ok": true,
            "reviewer_metadata_reader_parity_ok": false,
            "analytics_reader_parity_ok": true,
            "audit_reader_parity_ok": true,
            "replay_export_manifest_reader_parity_ok": true,
            "db_reader_parity_failures": ["reviewer_metadata: failed"],
            "active_vector_entries": 7,
            "accepted_current_derived_without_active_vector_entry": ["vector-a", "vector-b"],
            "invalid_active_vector_entries": 1,
            "blocking_gaps": [
                "missing_submission_ids_in_db=2",
                "reviewer_metadata_reader_parity=failed"
            ]
        }
    }"#,
    )
    .expect("valid reconciliation fixture");

    let lines = maintenance_reconciliation_lines(&value);

    assert_eq!(
        lines,
        vec![
            "  db reconciliation:".to_string(),
            "    submissions: files=3 db=2 missing_in_db=2 missing_in_files=0 status_mismatches=1".to_string(),
            "    derived: files=4 db=3 missing_in_db=1 missing_in_files=1 status_mismatches=1 hash_mismatches=1".to_string(),
            "    object refs: db=2 accepted_without_active_envelope=1 unreadable_active_envelope=1 hash_mismatched_active_envelope=1".to_string(),
            "    ledger/audit: file_credit_events=5 db_credit_events=4 file_audit_events=6 db_audit_events=6 db_retention_jobs=2 db_retention_items=4".to_string(),
            "    exports/tombstones: file_replay_manifests=1 db_export_manifests=2 db_replay_manifests=1 db_benchmark_manifests=0 db_ranker_manifests=1 db_other_manifests=0 db_export_items=3 file_revocation_tombstones=1 db_tombstones=1".to_string(),
            "    reader parity: contributor_credit=true reviewer_metadata=false analytics=true audit=true replay_export_manifests=true failures=1".to_string(),
            "    vectors: active=7 eligible_without_active=2 invalid_active=1".to_string(),
            "    blocking: gaps=2".to_string(),
        ]
    );
    let rendered = lines.join("\n");
    assert!(!rendered.contains("11111111-1111-1111-1111-111111111111"));
    assert!(!rendered.contains("33333333-3333-3333-3333-333333333333"));
}

#[test]
fn maintenance_reconciliation_lines_summarize_ledger_audit_gap_counts_without_ids() {
    let value = serde_json::json!({
        "db_reconciliation": {
            "file_credit_event_count": 5,
            "db_credit_event_count": 4,
            "missing_credit_event_ids_in_db": [
                "dddddddd-dddd-dddd-dddd-dddddddddddd"
            ],
            "missing_credit_event_ids_in_files": [],
            "file_audit_event_count": 6,
            "db_audit_event_count": 5,
            "missing_audit_event_ids_in_db": [
                "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee",
                "ffffffff-ffff-ffff-ffff-ffffffffffff"
            ],
            "missing_audit_event_ids_in_files": [
                "12121212-1212-1212-1212-121212121212"
            ]
        }
    });

    let lines = maintenance_reconciliation_lines(&value);

    assert_eq!(
        lines,
        vec![
            "  db reconciliation:".to_string(),
            "    ledger/audit: file_credit_events=5 db_credit_events=4 missing_credit_in_db=1 missing_credit_in_files=0 file_audit_events=6 db_audit_events=5 missing_audit_in_db=2 missing_audit_in_files=1".to_string(),
        ]
    );
    let rendered = lines.join("\n");
    assert!(!rendered.contains("dddddddd-dddd-dddd-dddd-dddddddddddd"));
    assert!(!rendered.contains("eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee"));
    assert!(!rendered.contains("12121212-1212-1212-1212-121212121212"));
}

#[test]
fn maintenance_reconciliation_lines_summarize_export_object_ref_gap_counts_without_ids() {
    let value = serde_json::json!({
        "db_reconciliation": {
            "file_replay_export_manifest_count": 1,
            "db_export_manifest_count": 2,
            "db_replay_export_manifest_count": 1,
            "db_benchmark_export_manifest_count": 0,
            "db_ranker_export_manifest_count": 1,
            "db_other_export_manifest_count": 0,
            "db_export_manifest_item_count": 3,
            "db_export_manifest_item_missing_object_ref_count": 2,
            "db_export_manifest_ids_with_missing_object_refs": [
                "abababab-abab-abab-abab-abababababab",
                "cdcdcdcd-cdcd-cdcd-cdcd-cdcdcdcdcdcd"
            ],
            "file_revocation_tombstone_count": 1,
            "db_tombstone_count": 1
        }
    });

    let lines = maintenance_reconciliation_lines(&value);

    assert_eq!(
        lines,
        vec![
            "  db reconciliation:".to_string(),
            "    exports/tombstones: file_replay_manifests=1 db_export_manifests=2 db_replay_manifests=1 db_benchmark_manifests=0 db_ranker_manifests=1 db_other_manifests=0 db_export_items=3 db_export_items_missing_object_refs=2 db_export_manifests_missing_object_refs=2 file_revocation_tombstones=1 db_tombstones=1".to_string(),
        ]
    );
    let rendered = lines.join("\n");
    assert!(!rendered.contains("abababab-abab-abab-abab-abababababab"));
    assert!(!rendered.contains("cdcdcdcd-cdcd-cdcd-cdcd-cdcdcdcdcdcd"));
}

#[test]
fn maintenance_request_body_includes_reconcile_flag() {
    let options = TraceCommonsMaintenanceOptions {
        purpose: "db-read-cutover".to_string(),
        dry_run: true,
        backfill_db_mirror: true,
        index_vectors: true,
        reconcile_db_mirror: true,
        verify_audit_chain: true,
        prune_export_cache: false,
        max_export_age_hours: Some(48),
        purge_expired_before: Some("2026-04-25T00:00:00Z".to_string()),
    };
    let body = trace_commons_maintenance_body(&options);

    assert_eq!(body["purpose"], "db-read-cutover");
    assert_eq!(body["dry_run"], true);
    assert_eq!(body["backfill_db_mirror"], true);
    assert_eq!(body["index_vectors"], true);
    assert_eq!(body["reconcile_db_mirror"], true);
    assert_eq!(body["verify_audit_chain"], true);
    assert_eq!(body["prune_export_cache"], false);
    assert_eq!(body["max_export_age_hours"], 48);
    assert_eq!(body["purge_expired_before"], "2026-04-25T00:00:00Z");
}

#[test]
fn maintenance_audit_chain_lines_summarize_without_hashes() {
    let value = serde_json::json!({
        "audit_chain": {
            "verified": false,
            "event_count": 4,
            "legacy_event_count": 1,
            "mismatch_count": 2,
            "last_event_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "failures": [
                "line 2: event_hash mismatch",
                "line 3: previous_event_hash mismatch"
            ],
            "db_mirror": {
                "verified": false,
                "event_count": 3,
                "legacy_event_count": 1,
                "payload_verified_event_count": 2,
                "payload_unverified_event_count": 1,
                "mismatch_count": 1,
                "last_event_hash": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "failures": [
                    "db row 3: previous_event_hash mismatch"
                ]
            }
        }
    });

    let lines = maintenance_audit_chain_lines(&value);
    assert_eq!(
        lines,
        vec![
            "  audit chain:".to_string(),
            "    status: verified=false events=4 legacy=1 mismatches=2".to_string(),
            "    failures: 2".to_string(),
            "    db mirror:".to_string(),
            "      status: verified=false events=3 legacy=1 payload_verified=2 payload_unverified=1 mismatches=1".to_string(),
            "      failures: 1".to_string(),
        ]
    );
    let rendered = lines.join("\n");
    assert!(!rendered.contains("sha256:aaaaaaaa"));
    assert!(!rendered.contains("sha256:bbbbbbbb"));
    assert!(!rendered.contains("line 2"));
    assert!(!rendered.contains("db row 3"));
}
