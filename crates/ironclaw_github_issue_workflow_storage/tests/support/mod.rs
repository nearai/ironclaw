#![allow(dead_code)]

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Duration, TimeZone, Utc};
use ironclaw_github_issue_workflow::{
    AdvanceWorkflowRunInput, ClaimRunnableWorkflowRunsInput, CreateOrGetProviderActionInput,
    CreateOrGetWorkflowRunInput, CreateOrGetWorkflowRunOutcome, CreateStageRunInput,
    GithubIssueRef, GithubIssueWorkflowEventType, GithubIssueWorkflowRepository,
    GithubIssueWorkflowRun, GithubProviderRef, InMemoryGithubIssueWorkflowRepository,
    ProviderActionKind, ProviderActionReconciliationStrategy, RecordWorkflowEventInput,
    UpsertProviderBindingInput, WorkflowEventEnvelope, WorkflowEventSourceKind,
    WorkflowIdempotencyKey, WorkflowRunTransition, WorkflowWorkerId,
};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use serde_json::json;

pub fn fixed_time(seconds: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(seconds, 0)
        .single()
        .expect("fixed test timestamp must be valid")
}

pub fn unique_suffix(name: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after unix epoch")
        .as_nanos();
    format!("{name}-{nanos}")
}

pub fn tenant(case: &str, suffix: u64) -> TenantId {
    TenantId::new(format!("tenant-{case}-{suffix}")).expect("valid tenant id")
}

pub fn user(case: &str, suffix: u64) -> UserId {
    UserId::new(format!("user-{case}-{suffix}")).expect("valid user id")
}

pub fn agent(case: &str, suffix: u64) -> AgentId {
    AgentId::new(format!("agent-{case}-{suffix}")).expect("valid agent id")
}

pub fn project(case: &str, suffix: u64) -> ProjectId {
    ProjectId::new(format!("project-{case}-{suffix}")).expect("valid project id")
}

pub fn issue(case: &str, number: u64) -> GithubIssueRef {
    GithubIssueRef {
        owner: "nearai".to_string(),
        repo: "ironclaw".to_string(),
        number,
        node_id: Some(format!("issue-node-{case}-{number}")),
        url: format!("https://github.com/nearai/ironclaw/issues/{number}"),
        default_branch: "main".to_string(),
    }
}

pub fn provider_ref(case: &str, provider_id: impl Into<String>) -> GithubProviderRef {
    GithubProviderRef {
        system: "github".to_string(),
        resource_type: "issue".to_string(),
        owner: "nearai".to_string(),
        repo: "ironclaw".to_string(),
        provider_id: format!("{}-{}", case, provider_id.into()),
        provider_url: Some("https://github.com/nearai/ironclaw/issues/42".to_string()),
    }
}

pub fn workflow_run_input(
    case: &str,
    tenant_id: TenantId,
    issue_ref: GithubIssueRef,
    now: chrono::DateTime<Utc>,
) -> CreateOrGetWorkflowRunInput {
    CreateOrGetWorkflowRunInput {
        tenant_id,
        creator_user_id: user(case, 1),
        agent_id: Some(agent(case, 1)),
        project_id: Some(project(case, 1)),
        provider_account_ref: None,
        issue_ref,
        workflow_policy_key: "github-bug-workflow".to_string(),
        workflow_policy_version: "2026-06-22".to_string(),
        now,
    }
}

pub async fn create_run(
    repository: &dyn GithubIssueWorkflowRepository,
    case: &str,
    tenant_id: TenantId,
    issue_ref: GithubIssueRef,
) -> GithubIssueWorkflowRun {
    match repository
        .create_or_get_workflow_run(workflow_run_input(
            case,
            tenant_id,
            issue_ref,
            fixed_time(10),
        ))
        .await
        .expect("create workflow run")
    {
        CreateOrGetWorkflowRunOutcome::Created { run }
        | CreateOrGetWorkflowRunOutcome::Existing { run } => run,
    }
}

pub fn event_input(
    case: &str,
    run: &GithubIssueWorkflowRun,
    key: &str,
    observed_at: chrono::DateTime<Utc>,
) -> RecordWorkflowEventInput {
    RecordWorkflowEventInput {
        workflow_run_id: run.workflow_run_id.clone(),
        workflow_event_type: GithubIssueWorkflowEventType::GithubIssueChanged,
        envelope: WorkflowEventEnvelope {
            source_kind: WorkflowEventSourceKind::Poller,
            source_delivery_id: None,
            provider: provider_ref(case, "issue-node-42"),
            observed_at,
            provider_updated_at: Some(observed_at),
            idempotency_key: WorkflowIdempotencyKey::from_trusted(key.to_string())
                .expect("valid idempotency key"),
            payload_schema: "github.issue.changed.v1".to_string(),
            payload: json!({ "issue_number": run.issue_ref.number, "key": key }),
        },
    }
}

pub fn worker(case: &str, suffix: u64) -> WorkflowWorkerId {
    WorkflowWorkerId::from_trusted(format!("worker-{case}-{suffix}")).expect("valid worker id")
}

pub fn provider_action_input(
    run: &GithubIssueWorkflowRun,
    key: WorkflowIdempotencyKey,
    now: chrono::DateTime<Utc>,
) -> CreateOrGetProviderActionInput {
    CreateOrGetProviderActionInput {
        workflow_run_id: run.workflow_run_id.clone(),
        stage_run_id: None,
        step_run_id: None,
        name: "claim-comment".to_string(),
        kind: ProviderActionKind::ClaimComment,
        idempotency_key: key,
        input_hash: "sha256:claim-comment-input".to_string(),
        stable_marker: Some("claim-marker".to_string()),
        reconciliation_strategy: ProviderActionReconciliationStrategy::ClaimCommentByMarker,
        now,
    }
}

pub fn claim_input(
    case: &str,
    tenant_id: TenantId,
    now: chrono::DateTime<Utc>,
    worker_suffix: u64,
) -> ClaimRunnableWorkflowRunsInput {
    ClaimRunnableWorkflowRunsInput {
        tenant_id,
        worker_id: worker(case, worker_suffix),
        now,
        lease_expires_at: now + Duration::seconds(60),
        limit: 10,
    }
}

pub fn advance_input(
    case: &str,
    run: &GithubIssueWorkflowRun,
    expected_workflow_run_version: i64,
    next_event_cursor: i64,
    now: chrono::DateTime<Utc>,
) -> AdvanceWorkflowRunInput {
    AdvanceWorkflowRunInput {
        workflow_run_id: run.workflow_run_id.clone(),
        worker_id: worker(case, 1),
        expected_workflow_run_version,
        expected_event_cursor: 0,
        next_event_cursor,
        transition: WorkflowRunTransition::default(),
        now,
    }
}

pub fn stage_input(run: &GithubIssueWorkflowRun) -> CreateStageRunInput {
    CreateStageRunInput {
        workflow_run_id: run.workflow_run_id.clone(),
        stage: ironclaw_github_issue_workflow::GithubIssueStage::Triage,
        now: fixed_time(20),
    }
}

pub fn binding_input(
    run: &GithubIssueWorkflowRun,
    provider_ref: GithubProviderRef,
    created_at: chrono::DateTime<Utc>,
) -> UpsertProviderBindingInput {
    UpsertProviderBindingInput {
        workflow_run_id: run.workflow_run_id.clone(),
        provider_ref,
        role: "primary".to_string(),
        created_by_provider_action_id: None,
        created_at,
    }
}

/// libSQL's native layer is unsafe under concurrent connection open/use within
/// a single process: running these contract tests in parallel threads
/// intermittently crashes the process (SIGSEGV/SIGABRT) or surfaces a
/// "bad parameter or other API misuse" error from libSQL. Production uses a
/// single connection per process, so this is purely a test-harness concern.
/// `cases()` acquires this lock and every `RepositoryCase` it returns holds a
/// clone for the test's full duration, so RepositoryCase-based tests run one at
/// a time within a binary (each is sub-millisecond, so the cost is negligible).
/// Each test calls `cases()` exactly once, so this cannot deadlock.
fn case_serial_lock() -> Arc<tokio::sync::Mutex<()>> {
    static LOCK: std::sync::OnceLock<Arc<tokio::sync::Mutex<()>>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

pub struct RepositoryCase {
    pub name: String,
    backend: RepositoryBackend,
    _serial_guard: Arc<tokio::sync::OwnedMutexGuard<()>>,
}

enum RepositoryBackend {
    InMemory(Arc<InMemoryGithubIssueWorkflowRepository>),
    #[cfg(feature = "libsql")]
    LibSql {
        path: String,
        _temp_dir: tempfile::TempDir,
    },
    #[cfg(feature = "postgres")]
    Postgres {
        filesystem: Arc<ironclaw_filesystem::PostgresRootFilesystem>,
    },
}

impl RepositoryCase {
    pub async fn cases(test_name: &str) -> Vec<Self> {
        let suffix = unique_suffix(test_name);
        let serial_guard = Arc::new(case_serial_lock().lock_owned().await);
        let mut cases = vec![Self {
            name: format!("in-memory-{suffix}"),
            backend: RepositoryBackend::InMemory(Arc::new(
                InMemoryGithubIssueWorkflowRepository::default(),
            )),
            _serial_guard: serial_guard.clone(),
        }];

        #[cfg(feature = "libsql")]
        {
            let dir = tempfile::tempdir().expect("tempdir");
            let db_path = dir.path().join(format!("{suffix}.db"));
            cases.push(Self {
                name: format!("libsql-{suffix}"),
                backend: RepositoryBackend::LibSql {
                    path: db_path.display().to_string(),
                    _temp_dir: dir,
                },
                _serial_guard: serial_guard.clone(),
            });
        }

        #[cfg(feature = "postgres")]
        if let Some(filesystem) = postgres_filesystem().await {
            cases.push(Self {
                name: format!("postgres-{suffix}"),
                backend: RepositoryBackend::Postgres { filesystem },
                _serial_guard: serial_guard.clone(),
            });
        }

        cases
    }

    pub async fn open(&self) -> Arc<dyn GithubIssueWorkflowRepository> {
        match &self.backend {
            RepositoryBackend::InMemory(repository) => repository.clone(),
            #[cfg(feature = "libsql")]
            RepositoryBackend::LibSql { path, .. } => Arc::new(
                ironclaw_github_issue_workflow_storage::RebornLibSqlGithubIssueWorkflowRepository::new(
                    libsql_filesystem(path).await,
                ),
            ),
            #[cfg(feature = "postgres")]
            RepositoryBackend::Postgres { filesystem } => Arc::new(
                ironclaw_github_issue_workflow_storage::RebornPostgresGithubIssueWorkflowRepository::new(
                    Arc::clone(filesystem),
                ),
            ),
        }
    }

    pub async fn reopen(&self) -> Arc<dyn GithubIssueWorkflowRepository> {
        self.open().await
    }
}

#[cfg(feature = "libsql")]
async fn libsql_filesystem(path: &str) -> Arc<ironclaw_filesystem::LibSqlRootFilesystem> {
    let db = Arc::new(
        libsql::Builder::new_local(path)
            .build()
            .await
            .expect("build libsql db"),
    );
    let filesystem = Arc::new(ironclaw_filesystem::LibSqlRootFilesystem::new(db));
    filesystem
        .run_migrations()
        .await
        .expect("run libsql filesystem migrations");
    filesystem
}

#[cfg(feature = "postgres")]
async fn postgres_filesystem() -> Option<Arc<ironclaw_filesystem::PostgresRootFilesystem>> {
    let url = match std::env::var("IRONCLAW_GITHUB_ISSUE_WORKFLOW_POSTGRES_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping postgres GitHub issue workflow storage contract: IRONCLAW_GITHUB_ISSUE_WORKFLOW_POSTGRES_URL not set"
            );
            return None;
        }
    };
    let config = match url.parse::<tokio_postgres::Config>() {
        Ok(config) => config,
        Err(error) => {
            eprintln!(
                "skipping postgres GitHub issue workflow storage contract: invalid url ({error})"
            );
            return None;
        }
    };
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .expect("postgres pool builds");
    if let Err(error) = pool.get().await {
        eprintln!(
            "skipping postgres GitHub issue workflow storage contract: database unavailable ({error})"
        );
        return None;
    }
    let filesystem = Arc::new(ironclaw_filesystem::PostgresRootFilesystem::new(pool));
    if let Err(error) = filesystem.run_migrations().await {
        eprintln!(
            "skipping postgres GitHub issue workflow storage contract: filesystem migrations failed ({error})"
        );
        return None;
    }
    Some(filesystem)
}
