# GitHub Bug Workflow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a first-party GitHub bug-fix workflow application that discovers project-scoped GitHub issues labeled `bug`, drives normal IronClaw stage turns, validates sealed structured stage results, and opens a draft pull request through idempotent provider actions.

**Architecture:** The workflow application lives above IronClaw's existing Reborn turn/runtime/product substrate. The only IronClaw platform primitive added for the MVP is a sealed first-party result sink, `builtin.workflow_report_stage_result`; GitHub lifecycle policy, issue selection, idempotency, provider writes, stage prompts, fan-out policy, and polling live in new workflow application/storage crates plus composition wiring.

**Tech Stack:** Rust 2024, tokio, async-trait, serde/serde_json, chrono, uuid, thiserror, IronClaw Reborn turns/threads/projects/host-runtime/filesystem, existing GitHub first-party extension capabilities. Specs: `docs/superpowers/specs/2026-06-15-github-bug-orchestration-design.md` and `docs/superpowers/specs/2026-06-15-github-bug-orchestration-overview.md`.

**Working dir:** `/Users/ben/near/ironclaw`.

## Global Constraints

- Do not commit any changes while executing this plan unless Ben explicitly reauthorizes commits in a later message. Each task ends with a status checkpoint instead of a commit.
- Keep GitHub bug lifecycle policy out of `ironclaw_agent_loop`, `ironclaw_turns`, `ironclaw_triggers`, `ironclaw_product_workflow`, and the GitHub WASM extension.
- Stage turns are normal scoped Reborn turns. Do not mint `TrustedInboundTurnRequest`, `TrustedTriggerSubmitRequest`, or trigger-owned trusted ingress from this workflow.
- Agents do not receive GitHub write capabilities for this workflow. They produce structured intent; workflow provider actions perform GitHub writes.
- Project-scoped product use must check `ironclaw_projects::ProjectRepository` access before scanning, submitting stage turns, or writing to GitHub.
- Workflow domain/product records may store scoped workspace refs and mount refs; do not expose raw host paths to model-visible context or product surfaces.
- Default child subagents are read-only for exploration/review/test-log analysis. Writable fan-out is not in this MVP; future writable fan-out must use workflow-managed child stage tasks with isolated workspace ownership.
- New durable persistence must support in-memory tests first and libSQL/PostgreSQL parity before production enablement.
- Preserve existing Reborn defaults. The workflow poller is disabled by default and only starts when explicitly enabled.
- Run `cargo fmt` after implementation tasks and use targeted tests first; expand to `cargo test -p ironclaw_architecture` after dependency or boundary changes.

---

## Boundary Summary

**Actual IronClaw platform/binary changes scoped by this plan:**

- Add a generic sealed first-party capability in `ironclaw_host_runtime`: `builtin.workflow_report_stage_result`.
- Add runtime/config/composition wiring so the Reborn binary can start the workflow app when configured.
- Add workflow capability/run-profile wiring in composition so stage turns see only the stage-appropriate tools.

**Workflow application changes scoped by this plan:**

- Add `ironclaw_github_issue_workflow` for domain, policy, ports, stage schemas, prompts, in-memory repository, polling, and provider action orchestration.
- Add `ironclaw_github_issue_workflow_storage` for durable adapters over IronClaw filesystem backends, with libSQL/PostgreSQL features.
- Add `crates/ironclaw_reborn_composition/src/github_issue_workflow.rs` as the adapter layer over existing IronClaw services.

**Explicitly not changed in the MVP:**

- No agent-loop rewrite.
- No new subagent runtime.
- No webhook listener.
- No generic orchestration kernel.
- No GitHub WASM extension policy rewrite.
- No user-facing `ironclaw_triggers` schedule for the MVP poller.
- No direct GitHub write tools in model-visible stage profiles.

## File Structure

### New workflow crate

- Create `crates/ironclaw_github_issue_workflow/Cargo.toml` — application crate manifest.
- Create `crates/ironclaw_github_issue_workflow/src/lib.rs` — public module exports.
- Create `crates/ironclaw_github_issue_workflow/src/error.rs` — sanitized workflow error taxonomy.
- Create `crates/ironclaw_github_issue_workflow/src/ids.rs` — strongly typed workflow ids and idempotency keys.
- Create `crates/ironclaw_github_issue_workflow/src/domain.rs` — workflow run/state/ref records.
- Create `crates/ironclaw_github_issue_workflow/src/config.rs` — project-scoped workflow configuration.
- Create `crates/ironclaw_github_issue_workflow/src/workflow_events.rs` — ingress event envelopes, typed payloads, idempotency key helpers.
- Create `crates/ironclaw_github_issue_workflow/src/repository.rs` — atomic repository trait and DTOs.
- Create `crates/ironclaw_github_issue_workflow/src/in_memory.rs` — in-memory repository used by unit/integration tests and local-dev disabled-by-default wiring.
- Create `crates/ironclaw_github_issue_workflow/src/provider_bindings.rs` — provider resource routing and echo suppression model.
- Create `crates/ironclaw_github_issue_workflow/src/provider_actions.rs` — idempotent provider action records and runner contracts.
- Create `crates/ironclaw_github_issue_workflow/src/stages.rs` — stage run records, stage identity, stage turn request/response.
- Create `crates/ironclaw_github_issue_workflow/src/stage_schemas.rs` — strict stage-result validators.
- Create `crates/ironclaw_github_issue_workflow/src/snapshots.rs` — engineered stage snapshots and snapshot hashing.
- Create `crates/ironclaw_github_issue_workflow/src/prompts.rs` — prompt references and prompt rendering input contracts.
- Create `crates/ironclaw_github_issue_workflow/src/policy.rs` — deterministic workflow policy tick.
- Create `crates/ironclaw_github_issue_workflow/src/poller.rs` — internal cron-style poller over the workflow ports.
- Create `crates/ironclaw_github_issue_workflow/src/ports.rs` — GitHub, stage-turn, workspace, project-access, clock ports.
- Create `crates/ironclaw_github_issue_workflow/src/testing.rs` — test fakes behind `test-support`.
- Create `crates/ironclaw_github_issue_workflow/prompts/github_bug/triage.v1.md`.
- Create `crates/ironclaw_github_issue_workflow/prompts/github_bug/planning.v1.md`.
- Create `crates/ironclaw_github_issue_workflow/prompts/github_bug/implementation.v1.md`.
- Create `crates/ironclaw_github_issue_workflow/prompts/github_bug/pr_synthesis.v1.md`.
- Create `crates/ironclaw_github_issue_workflow/prompts/github_bug/ci_repair.v1.md`.
- Create `crates/ironclaw_github_issue_workflow/prompts/github_bug/review_response.v1.md`.
- Create `crates/ironclaw_github_issue_workflow/tests/domain_contract.rs`.
- Create `crates/ironclaw_github_issue_workflow/tests/repository_contract.rs`.
- Create `crates/ironclaw_github_issue_workflow/tests/provider_action_contract.rs`.
- Create `crates/ironclaw_github_issue_workflow/tests/policy_contract.rs`.
- Create `crates/ironclaw_github_issue_workflow/tests/stage_result_contract.rs`.
- Create `crates/ironclaw_github_issue_workflow/tests/prompt_snapshot_contract.rs`.
- Create `crates/ironclaw_github_issue_workflow/tests/poller_contract.rs`.
- Create `crates/ironclaw_github_issue_workflow/tests/workspace_stage_contract.rs`.
- Create `crates/ironclaw_github_issue_workflow/tests/pr_lifecycle_contract.rs`.
- Create `crates/ironclaw_github_issue_workflow/tests/webhook_readiness_contract.rs`.

### New workflow storage crate

- Create `crates/ironclaw_github_issue_workflow_storage/Cargo.toml` — durable storage manifest with `libsql` and `postgres` features.
- Create `crates/ironclaw_github_issue_workflow_storage/src/lib.rs` — storage exports.
- Create `crates/ironclaw_github_issue_workflow_storage/src/filesystem_repository.rs` — durable repository adapter over `ScopedFilesystem`.
- Create `crates/ironclaw_github_issue_workflow_storage/src/filesystem_repository/path.rs` — key/path derivation.
- Create `crates/ironclaw_github_issue_workflow_storage/tests/durable_repository_contract.rs`.
- Create `crates/ironclaw_github_issue_workflow_storage/tests/support/mod.rs`.

### Host-runtime sealed result primitive

- Create `crates/ironclaw_host_runtime/src/first_party_tools/workflow_result.rs` — sealed workflow result capability and sink trait.
- Modify `crates/ironclaw_host_runtime/src/first_party_tools/mod.rs` — register manifest, exports, and sink-backed handler registration function.
- Modify `crates/ironclaw_host_runtime/src/first_party_tools/schemas.rs` — publish input schema for `schemas/builtin/workflow-report-stage-result.input.v1.json`.
- Modify `crates/ironclaw_host_runtime/src/lib.rs` — re-export `WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID`, `WorkflowStageResultSink`, and related DTOs.
- Create `crates/ironclaw_host_runtime/tests/workflow_result_tool_contract.rs`.

### Reborn config/composition/runtime wiring

- Modify root `Cargo.toml` — add new crates to workspace members.
- Modify `crates/ironclaw_reborn_config/src/config_file.rs` — add `[github_issue_workflow]` boot config section and parse tests.
- Modify `crates/ironclaw_reborn_config/src/lib.rs` — export the config section.
- Modify `crates/ironclaw_reborn_composition/Cargo.toml` — add optional deps/features for the workflow crates and storage crate.
- Create `crates/ironclaw_reborn_composition/src/github_issue_workflow.rs` — composition adapters, stage submitter, project access adapter, poller builder.
- Modify `crates/ironclaw_reborn_composition/src/lib.rs` — add `mod github_issue_workflow;`.
- Modify `crates/ironclaw_reborn_composition/src/runtime_input.rs` — add `GithubIssueWorkflowSettings`.
- Modify `crates/ironclaw_reborn_composition/src/runtime.rs` — start/stop workflow poller, add readiness bit, wire sealed result sink into host runtime handlers.
- Modify `crates/ironclaw_reborn_composition/src/factory.rs` — instantiate durable/in-memory workflow repositories and expose them to runtime graph.
- Modify `crates/ironclaw_reborn_composition/src/local_dev_capability_policy.toml` — add the result capability grant; stage-only visibility is enforced by workflow run-profile capability filtering.
- Create `crates/ironclaw_reborn_composition/tests/github_issue_workflow_stage_turn.rs`.
- Create `crates/ironclaw_reborn_composition/tests/github_issue_workflow_capabilities.rs`.
- Create `crates/ironclaw_reborn_composition/tests/github_issue_workflow_provider.rs`.
- Create `crates/ironclaw_reborn_composition/tests/github_issue_workflow_runtime.rs`.
- Create `crates/ironclaw_reborn_composition/tests/github_issue_workflow_smoke.rs`.

### Existing GitHub provider assets

- Read but do not change for MVP unless a contract gap is discovered:
  - `crates/ironclaw_first_party_extensions/assets/github/manifest.toml`
  - `tools-src/github/src/lib.rs`
  - `crates/ironclaw_host_runtime/tests/github_wasm_runtime_contract.rs`

## Core Interfaces

The implementation should converge on these application-facing interfaces.

```rust
#[async_trait::async_trait]
pub trait GithubIssueWorkflowRepository: Send + Sync {
    async fn create_or_get_workflow_run(
        &self,
        input: CreateOrGetWorkflowRunInput,
    ) -> Result<CreateOrGetWorkflowRunOutcome, GithubIssueWorkflowError>;

    async fn record_workflow_event(
        &self,
        input: RecordWorkflowEventInput,
    ) -> Result<RecordWorkflowEventOutcome, GithubIssueWorkflowError>;

    async fn claim_runnable_workflow_runs(
        &self,
        input: ClaimRunnableWorkflowRunsInput,
    ) -> Result<Vec<GithubIssueWorkflowRun>, GithubIssueWorkflowError>;

    async fn renew_workflow_run_lease(
        &self,
        input: RenewWorkflowRunLeaseInput,
    ) -> Result<LeaseRenewalOutcome, GithubIssueWorkflowError>;

    async fn advance_event_cursor_and_transition(
        &self,
        input: AdvanceWorkflowRunInput,
    ) -> Result<TransitionOutcome, GithubIssueWorkflowError>;

    async fn create_stage_run(
        &self,
        input: CreateStageRunInput,
    ) -> Result<CreateStageRunOutcome, GithubIssueWorkflowError>;

    async fn accept_stage_result(
        &self,
        input: AcceptStageResultInput,
    ) -> Result<AcceptStageResultOutcome, GithubIssueWorkflowError>;

    async fn create_or_get_provider_action(
        &self,
        input: CreateOrGetProviderActionInput,
    ) -> Result<GithubIssueProviderActionRecord, GithubIssueWorkflowError>;

    async fn upsert_provider_binding(
        &self,
        input: UpsertProviderBindingInput,
    ) -> Result<GithubIssueProviderBinding, GithubIssueWorkflowError>;
}
```

```rust
#[async_trait::async_trait]
pub trait StageTurnSubmitter: Send + Sync {
    async fn submit_stage_turn(
        &self,
        request: SubmitStageTurnRequest,
    ) -> Result<SubmitStageTurnOutcome, GithubIssueWorkflowError>;
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StageTurnIdentity {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub stage_run_id: GithubIssueStageRunId,
    pub stage: GithubIssueStage,
    pub attempt: u32,
    pub workflow_policy_version: String,
}
```

```rust
#[async_trait::async_trait]
pub trait GithubIssueWorkflowPort: Send + Sync {
    async fn search_open_bug_issues(
        &self,
        request: SearchOpenBugIssuesRequest,
    ) -> Result<Vec<GithubIssueObservation>, GithubIssueWorkflowError>;

    async fn read_issue(
        &self,
        request: ReadGithubIssueRequest,
    ) -> Result<GithubIssueObservation, GithubIssueWorkflowError>;

    async fn list_issue_comments(
        &self,
        request: ListGithubIssueCommentsRequest,
    ) -> Result<Vec<GithubCommentObservation>, GithubIssueWorkflowError>;

    async fn create_issue_comment(
        &self,
        request: CreateGithubIssueCommentRequest,
    ) -> Result<GithubCommentRef, GithubIssueWorkflowError>;

    async fn create_draft_pull_request(
        &self,
        request: CreateDraftPullRequestRequest,
    ) -> Result<GithubPullRequestRef, GithubIssueWorkflowError>;

    async fn read_pull_request_lifecycle(
        &self,
        request: ReadPullRequestLifecycleRequest,
    ) -> Result<GithubPullRequestLifecycleObservation, GithubIssueWorkflowError>;
}
```

```rust
#[async_trait::async_trait]
pub trait WorkflowProjectAccess: Send + Sync {
    async fn assert_workflow_project_access(
        &self,
        request: WorkflowProjectAccessRequest,
    ) -> Result<(), GithubIssueWorkflowError>;
}
```

## Task 1: Scaffold Workflow Domain Crate

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/ironclaw_github_issue_workflow/Cargo.toml`
- Create: `crates/ironclaw_github_issue_workflow/src/lib.rs`
- Create: `crates/ironclaw_github_issue_workflow/src/error.rs`
- Create: `crates/ironclaw_github_issue_workflow/src/ids.rs`
- Create: `crates/ironclaw_github_issue_workflow/src/domain.rs`
- Create: `crates/ironclaw_github_issue_workflow/src/config.rs`
- Test: `crates/ironclaw_github_issue_workflow/tests/domain_contract.rs`

**Interfaces:**
- Produces: `GithubIssueWorkflowRun`, `GithubIssueWorkflowState`, `GithubIssueWorkflowConfig`, `GithubIssueStage`, `GithubIssueWorkflowError`, typed ids.
- Consumes: `TenantId`, `UserId`, `AgentId`, `ProjectId`, `ThreadId` from `ironclaw_host_api`.

- [ ] **Step 1: Add the crate to the workspace**

Add `"crates/ironclaw_github_issue_workflow"` to the root `Cargo.toml` workspace `members` array next to the other `ironclaw_*` crates.

- [ ] **Step 2: Create `crates/ironclaw_github_issue_workflow/Cargo.toml`**

```toml
[package]
name = "ironclaw_github_issue_workflow"
version = "0.1.0"
edition = "2024"
rust-version = "1.92"
description = "GitHub issue automation workflow application for IronClaw Reborn"
authors = ["NEAR AI <support@near.ai>"]
license = "MIT OR Apache-2.0"
homepage = "https://github.com/nearai/ironclaw"
repository = "https://github.com/nearai/ironclaw"
publish = false

[features]
default = []
test-support = []

[dependencies]
async-trait = "0.1"
chrono = { version = "0.4", features = ["serde"] }
ironclaw_host_api = { path = "../ironclaw_host_api", version = "0.1.0" }
ironclaw_threads = { path = "../ironclaw_threads" }
ironclaw_turns = { path = "../ironclaw_turns", version = "0.1.0" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
thiserror = "2"
tokio = { version = "1", features = ["sync", "time"] }
uuid = { version = "1", features = ["v4", "v5", "serde"] }

[dev-dependencies]
tokio = { version = "1", features = ["macros", "rt", "sync", "time"] }
```

- [ ] **Step 3: Create module exports in `src/lib.rs`**

```rust
#![forbid(unsafe_code)]

mod config;
mod domain;
mod error;
mod ids;
mod in_memory;
mod policy;
mod poller;
mod ports;
mod prompts;
mod provider_actions;
mod provider_bindings;
mod repository;
mod snapshots;
mod stage_schemas;
mod stages;
mod workflow_events;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub use config::*;
pub use domain::*;
pub use error::*;
pub use ids::*;
pub use in_memory::InMemoryGithubIssueWorkflowRepository;
pub use policy::*;
pub use poller::*;
pub use ports::*;
pub use prompts::*;
pub use provider_actions::*;
pub use provider_bindings::*;
pub use repository::*;
pub use snapshots::*;
pub use stage_schemas::*;
pub use stages::*;
pub use workflow_events::*;
```

- [ ] **Step 4: Implement `ids.rs` with bounded string ids**

Define `GithubIssueWorkflowRunId`, `GithubIssueStageRunId`, `GithubIssueProviderActionId`, `GithubIssueProviderBindingId`, `WorkflowStepRunId`, `WorkflowWorkerId`, and `WorkflowIdempotencyKey`. Each wraps `String`, implements `Clone + Debug + PartialEq + Eq + Hash + Serialize + Deserialize + Display`, has `new()` for UUID-backed ids where appropriate, and has `from_trusted(String)` only for deterministic keys generated inside the workflow crate.

- [ ] **Step 5: Implement `domain.rs`**

Define:

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubIssueWorkflowRunStatus {
    Active,
    Blocked,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubIssueWorkflowMode {
    New,
    Claimed,
    Triage,
    Planning,
    Implementation,
    PrSynthesis,
    PrOpen,
    CiRepair,
    ReviewResponse,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubIssueStage {
    Triage,
    Planning,
    Implementation,
    PrSynthesis,
    CiRepair,
    ReviewResponse,
}
```

Also define `GithubIssueRef`, `GithubPullRequestRef`, `WorkflowWorkspaceRef`, `GithubIssueWorkflowState`, and `GithubIssueWorkflowRun` using the shape from the design spec. `project_id` remains `Option<ProjectId>` in the record for transitional local-dev support, but production config validation in Task 12 must require it.

- [ ] **Step 6: Implement `config.rs`**

Define `GithubIssueWorkflowConfig`, `GithubRepositorySelector`, `GithubIssueCandidateSelector`, and `GithubProviderAccountRef`. Use project-scoped config:

```rust
pub struct GithubIssueWorkflowConfig {
    pub tenant_id: TenantId,
    pub project_id: ProjectId,
    pub owner_user_id: UserId,
    pub repositories: Vec<GithubRepositorySelector>,
    pub candidate_selector: GithubIssueCandidateSelector,
    pub max_active_runs_per_repo: u32,
    pub default_run_profile: String,
    pub provider_account_ref: GithubProviderAccountRef,
}
```

This task does not mutate `ProjectRecord.metadata`; config lives in the workflow repository keyed by `(tenant_id, project_id)`.

- [ ] **Step 7: Write domain contract tests**

Tests:

- `workflow_run_key_is_stable_for_issue_ref`
- `workflow_state_mode_is_distinct_from_run_status`
- `project_scoped_config_rejects_empty_repositories`
- `idempotency_key_rejects_empty_and_overlong_values`

Run: `cargo test -p ironclaw_github_issue_workflow domain_contract`.
Expected: pass after implementation.

- [ ] **Step 8: Status checkpoint**

Report: crate scaffolded, domain/config contract tests pass, no commits made.

## Task 2: Workflow Events, Repository Trait, And In-Memory Atomicity

**Files:**
- Create: `crates/ironclaw_github_issue_workflow/src/workflow_events.rs`
- Create: `crates/ironclaw_github_issue_workflow/src/repository.rs`
- Create: `crates/ironclaw_github_issue_workflow/src/in_memory.rs`
- Test: `crates/ironclaw_github_issue_workflow/tests/repository_contract.rs`

**Interfaces:**
- Consumes: domain ids/types from Task 1.
- Produces: `WorkflowEventEnvelope`, `GithubIssueWorkflowEvent`, `GithubIssueWorkflowRepository`, `InMemoryGithubIssueWorkflowRepository`.

- [ ] **Step 1: Implement workflow event envelope and payloads**

Create:

```rust
pub struct WorkflowEventEnvelope<TPayload> {
    pub source_kind: WorkflowEventSourceKind,
    pub source_delivery_id: Option<String>,
    pub provider: GithubProviderRef,
    pub observed_at: chrono::DateTime<chrono::Utc>,
    pub provider_updated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub idempotency_key: WorkflowIdempotencyKey,
    pub payload_schema: String,
    pub payload: TPayload,
}
```

Create `WorkflowEventSourceKind` with `Poller`, `GithubWebhook`, `BenchmarkWebhook`, `ManualOperator`, and `WorkflowInternal`. Create typed payload structs for the initial event set in the design:

- `GithubIssueDiscoveredPayload`
- `GithubIssueChangedPayload`
- `GithubIssueClosedPayload`
- `GithubPullRequestOpenedPayload`
- `GithubPullRequestUpdatedPayload`
- `GithubChecksChangedPayload`
- `GithubReviewCommentCreatedPayload`
- `StageCompletedPayload`
- `ProviderActionChangedPayload`
- `WorkflowRunBlockedPayload`

- [ ] **Step 2: Implement deterministic idempotency key builders**

Functions:

```rust
pub fn issue_discovered_key(issue: &GithubIssueRef) -> WorkflowIdempotencyKey;
pub fn issue_changed_key(issue: &GithubIssueRef, provider_updated_at: Option<DateTime<Utc>>) -> WorkflowIdempotencyKey;
pub fn pr_opened_key(pr: &GithubPullRequestRef) -> WorkflowIdempotencyKey;
pub fn checks_changed_key(head_sha: &str, suite_or_run_id: &str, conclusion: &str) -> WorkflowIdempotencyKey;
pub fn review_comment_created_key(comment_node_id: &str) -> WorkflowIdempotencyKey;
pub fn stage_result_reported_key(stage_run_id: &GithubIssueStageRunId, schema_version: &str) -> WorkflowIdempotencyKey;
```

- [ ] **Step 3: Implement the repository trait exactly as listed in Core Interfaces**

Atomic methods must return outcome enums rather than loose booleans:

```rust
pub enum RecordWorkflowEventOutcome {
    Recorded { event: GithubIssueWorkflowEvent },
    Duplicate { existing: GithubIssueWorkflowEvent },
    Superseded { existing: GithubIssueWorkflowEvent },
}

pub enum TransitionOutcome {
    Applied { run: GithubIssueWorkflowRun },
    VersionConflict { current: GithubIssueWorkflowRun },
    NotLeaseOwner,
    Terminal,
}
```

- [ ] **Step 4: Implement `InMemoryGithubIssueWorkflowRepository`**

Use one `tokio::sync::Mutex<InMemoryState>` and keep all repository methods atomic inside that mutex. The in-memory repository must enforce:

- unique workflow run key per tenant;
- unique event idempotency key per workflow run;
- monotonically increasing event sequence per workflow run;
- compare-and-swap on `workflow_run_version` and `event_cursor`;
- lease owner/expiry checks;
- unique active stage per workflow run;
- provider action uniqueness by `(workflow_run_id, idempotency_key)`;
- provider binding uniqueness by provider resource and role.

- [ ] **Step 5: Write repository contract tests**

Tests:

- `create_or_get_workflow_run_is_idempotent_per_tenant`
- `record_workflow_event_dedupes_by_run_and_key`
- `record_workflow_event_sequences_are_monotonic`
- `claim_runnable_workflow_runs_honors_lease_expiry`
- `advance_event_cursor_requires_expected_version`
- `create_stage_run_rejects_second_active_stage`
- `create_or_get_provider_action_dedupes_by_input_hash`
- `upsert_provider_binding_routes_by_provider_ref`

Run: `cargo test -p ironclaw_github_issue_workflow repository_contract`.
Expected: all tests pass.

- [ ] **Step 6: Status checkpoint**

Report: in-memory repository contracts pass, no commits made.

## Task 3: Provider Bindings, Provider Actions, And Claim Protocol

**Files:**
- Create: `crates/ironclaw_github_issue_workflow/src/provider_bindings.rs`
- Create: `crates/ironclaw_github_issue_workflow/src/provider_actions.rs`
- Test: `crates/ironclaw_github_issue_workflow/tests/provider_action_contract.rs`

**Interfaces:**
- Consumes: repository trait and GitHub provider port from Tasks 1-2.
- Produces: `GithubIssueProviderActionRunner`, provider action records, claim comment protocol.

- [ ] **Step 1: Implement provider binding model**

Define `GithubIssueProviderBinding` exactly as the design shape:

```rust
pub struct GithubIssueProviderBinding {
    pub binding_id: GithubIssueProviderBindingId,
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub system: String,
    pub resource_type: String,
    pub role: String,
    pub owner: String,
    pub repo: String,
    pub provider_id: String,
    pub provider_url: Option<String>,
    pub created_by_provider_action_id: Option<GithubIssueProviderActionId>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
```

Implement helpers:

```rust
pub fn issue_binding_ref(issue: &GithubIssueRef) -> GithubProviderBindingRef;
pub fn claim_comment_binding_ref(issue: &GithubIssueRef, marker: &str) -> GithubProviderBindingRef;
pub fn primary_pr_binding_ref(pr: &GithubPullRequestRef) -> GithubProviderBindingRef;
```

- [ ] **Step 2: Implement provider action record model**

Define `GithubIssueProviderActionRecord`, `ProviderActionStatus`, `ProviderActionKind`, and `ProviderActionReconciliationStrategy`. The MVP strategies are:

- `ClaimCommentByMarker`
- `IssueCommentByMarker`
- `BranchByNameAndHeadSha`
- `DraftPullRequestByHeadBranchAndMarker`
- `ReviewReplyByParentCommentAndMarker`

- [ ] **Step 3: Implement stable marker generation**

Functions:

```rust
pub fn stable_claim_marker(run_id: &GithubIssueWorkflowRunId) -> String;
pub fn stable_pr_marker(run_id: &GithubIssueWorkflowRunId) -> String;
pub fn stable_issue_comment_marker(action_id: &GithubIssueProviderActionId) -> String;
```

Marker strings must be HTML comments so they survive GitHub body rendering:

```text
<!-- ironclaw:github-bug-workflow:<kind>:<id> -->
```

- [ ] **Step 4: Implement `GithubIssueProviderActionRunner`**

The runner claims a pending action lease, calls `GithubIssueWorkflowPort`, records success/failure/reconciliation state, and writes a provider binding on success. It must not call GitHub directly; only the port is allowed.

- [ ] **Step 5: Write provider action tests**

Tests:

- `claim_comment_uses_stable_marker_and_records_binding`
- `duplicate_claim_comment_replays_existing_action`
- `ambiguous_claim_comment_enters_needs_reconciliation`
- `self_authored_comment_with_known_marker_is_echo_suppressed`
- `provider_write_failure_is_sanitized`

Run: `cargo test -p ironclaw_github_issue_workflow provider_action_contract`.
Expected: all tests pass.

- [ ] **Step 6: Status checkpoint**

Report: provider action contracts pass, no commits made.

## Task 4: Workflow Policy Vertical Slice With Fake Ports

**Files:**
- Create: `crates/ironclaw_github_issue_workflow/src/policy.rs`
- Create: `crates/ironclaw_github_issue_workflow/src/ports.rs`
- Create: `crates/ironclaw_github_issue_workflow/src/stages.rs`
- Test: `crates/ironclaw_github_issue_workflow/tests/policy_contract.rs`

**Interfaces:**
- Consumes: repository/provider action contracts.
- Produces: deterministic `GithubIssueWorkflowPolicy::tick`.

- [ ] **Step 1: Define ports**

In `ports.rs`, define:

```rust
#[async_trait::async_trait]
pub trait WorkflowClock: Send + Sync {
    fn now(&self) -> chrono::DateTime<chrono::Utc>;
}

#[async_trait::async_trait]
pub trait WorkflowWorkspaceManager: Send + Sync {
    async fn prepare_workspace(
        &self,
        request: PrepareWorkflowWorkspaceRequest,
    ) -> Result<PrepareWorkflowWorkspaceOutcome, GithubIssueWorkflowError>;
}
```

Use the `GithubIssueWorkflowPort`, `StageTurnSubmitter`, and `WorkflowProjectAccess` interfaces from Core Interfaces.

- [ ] **Step 2: Define stage run model**

In `stages.rs`, define `GithubIssueStageRun`, `StageRunStatus`, `StageTurnIdentity`, `SubmitStageTurnRequest`, `SubmitStageTurnOutcome`, `WorkflowPromptContentRef`, and `WorkflowWorkspaceMountRef`.

`StageTurnIdentity` must derive deterministic:

- thread id seed;
- source binding ref;
- reply target binding ref;
- turn idempotency key;
- completion nonce.

- [ ] **Step 3: Implement policy tick**

Create:

```rust
pub struct GithubIssueWorkflowPolicy<P> {
    ports: P,
    policy_version: String,
}

impl<P> GithubIssueWorkflowPolicy<P>
where
    P: GithubIssueWorkflowPolicyPorts,
{
    pub async fn tick(
        &self,
        run: GithubIssueWorkflowRun,
    ) -> Result<WorkflowPolicyTickOutcome, GithubIssueWorkflowError>;
}
```

The initial state transitions:

```text
new + github.issue.discovered
  -> provider action: claim_issue
  -> mode claimed
  -> stage run triage

claimed/triage + stage.triage.completed
  -> stage run planning

planning + stage.plan.completed
  -> workspace prepare
  -> stage run implementation
```

- [ ] **Step 4: Keep side effects behind step records**

Create `WorkflowStepRun` and `WorkflowStepStatus`. Every policy side effect must go through a named step with deterministic `idempotency_key` and `input_hash`. Completed step results must replay without re-running the side effect.

- [ ] **Step 5: Write policy tests**

Tests:

- `issue_discovered_claims_then_starts_triage_once`
- `policy_tick_replays_completed_claim_step_without_second_comment`
- `triage_completion_starts_planning_stage`
- `planning_completion_prepares_workspace_then_starts_implementation`
- `project_access_denial_blocks_run_without_stage_submission`
- `turn_submission_busy_keeps_stage_active_without_duplicate_submit`

Run: `cargo test -p ironclaw_github_issue_workflow policy_contract`.
Expected: all tests pass.

- [ ] **Step 6: Status checkpoint**

Report: fake-port workflow policy vertical slice works, no commits made.

## Task 5: Sealed Structured Result Capability In Host Runtime

**Files:**
- Create: `crates/ironclaw_host_runtime/src/first_party_tools/workflow_result.rs`
- Modify: `crates/ironclaw_host_runtime/src/first_party_tools/mod.rs`
- Modify: `crates/ironclaw_host_runtime/src/first_party_tools/schemas.rs`
- Modify: `crates/ironclaw_host_runtime/src/lib.rs`
- Test: `crates/ironclaw_host_runtime/tests/workflow_result_tool_contract.rs`

**Interfaces:**
- Produces: generic platform primitive `WorkflowStageResultSink`.
- Consumes: no GitHub workflow crate types. This must remain generic host-runtime API.

- [ ] **Step 1: Add `workflow_result.rs`**

Define:

```rust
pub const WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID: &str =
    "builtin.workflow_report_stage_result";

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ReportWorkflowStageResultInput {
    pub workflow_run_id: String,
    pub stage_run_id: String,
    pub turn_run_id: String,
    pub stage: String,
    pub schema_version: String,
    pub completion_nonce: String,
    pub result: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkflowStageResultAck {
    pub accepted: bool,
    pub duplicate: bool,
    pub stage_run_id: String,
}

#[async_trait::async_trait]
pub trait WorkflowStageResultSink: Send + Sync {
    async fn report_stage_result(
        &self,
        input: ReportWorkflowStageResultInput,
    ) -> Result<WorkflowStageResultAck, WorkflowStageResultSinkError>;
}
```

Define `WorkflowStageResultSinkError` with sanitized variants:

- `InvalidInput { reason: String }`
- `MismatchedBinding`
- `StaleAttempt`
- `StageNotActive`
- `ValidationFailed { reason: String }`
- `Unavailable`

- [ ] **Step 2: Add manifest**

Create `workflow_result::manifest()` using `first_party_capability_manifest` with:

- capability id: `builtin.workflow_report_stage_result`;
- effects: `vec![EffectKind::DispatchCapability]`;
- default permission: `PermissionMode::Allow`;
- resource profile: standard first-party `resource_profile()`;
- input schema ref: `schemas/builtin/workflow-report-stage-result.input.v1.json`.

- [ ] **Step 3: Add handler insertion function**

In `workflow_result.rs`, add:

```rust
pub(super) fn insert_handler(
    registry: &mut FirstPartyCapabilityRegistry,
    sink: Arc<dyn WorkflowStageResultSink>,
) -> Result<(), HostApiError>;
```

The handler must:

- bound input size with `bounded_input_size`;
- deserialize `ReportWorkflowStageResultInput`;
- call `sink.report_stage_result`;
- return `WorkflowStageResultAck` as JSON;
- never include raw result payload in errors.

- [ ] **Step 4: Wire host-runtime module exports**

In `first_party_tools/mod.rs`:

- add `mod workflow_result;`;
- export `WORKFLOW_REPORT_STAGE_RESULT_CAPABILITY_ID`, `WorkflowStageResultSink`, `ReportWorkflowStageResultInput`, `WorkflowStageResultAck`;
- add `workflow_result::manifest()?` to `builtin_first_party_package()`;
- add `pub fn builtin_first_party_handlers_with_workflow_stage_result_sink(...)` that starts from `builtin_first_party_handlers(trigger_repository)` and then calls `workflow_result::insert_handler`.

Keep existing `builtin_first_party_handlers(...)` behavior unchanged for callers that do not wire this workflow.

- [ ] **Step 5: Add schema**

In `first_party_tools/schemas.rs`, add a match arm for `schemas/builtin/workflow-report-stage-result.input.v1.json` with required fields:

```json
["workflow_run_id", "stage_run_id", "turn_run_id", "stage", "schema_version", "completion_nonce", "result"]
```

Set `additionalProperties: false`.

- [ ] **Step 6: Write host-runtime tests**

Tests:

- `workflow_result_manifest_is_host_bundled_and_schema_resolves`
- `workflow_result_handler_forwards_to_sink`
- `workflow_result_handler_rejects_invalid_json_without_calling_sink`
- `workflow_result_handler_sanitizes_validation_failure`
- `default_builtin_handlers_do_not_register_workflow_result_sink`
- `workflow_sink_handlers_registers_result_capability`

Run: `cargo test -p ironclaw_host_runtime --test workflow_result_tool_contract`.
Expected: all tests pass.

- [ ] **Step 7: Status checkpoint**

Report: sealed result platform primitive added and tested, no commits made.

## Task 6: Stage Result Validation And Sink Adapter

**Files:**
- Create: `crates/ironclaw_github_issue_workflow/src/stage_schemas.rs`
- Modify: `crates/ironclaw_github_issue_workflow/src/stages.rs`
- Test: `crates/ironclaw_github_issue_workflow/tests/stage_result_contract.rs`

**Interfaces:**
- Consumes: generic `WorkflowStageResultSink` DTOs from host-runtime.
- Produces: GitHub-specific stage validators and `GithubWorkflowStageResultSink`.

- [ ] **Step 1: Define stage result envelope**

Create:

```rust
pub struct StageResultEnvelope {
    pub outcome: StageResultOutcome,
    pub summary: String,
    pub evidence: Vec<StageEvidence>,
    pub next_actions: Vec<String>,
    pub payload: serde_json::Value,
}

pub enum StageResultOutcome {
    Completed,
    NeedsHuman,
    GaveUp,
    ExhaustedTurns,
    NotProduced,
}
```

- [ ] **Step 2: Implement validators per stage**

Functions:

```rust
pub fn validate_stage_result(
    stage: GithubIssueStage,
    schema_version: &str,
    value: serde_json::Value,
) -> Result<ValidatedStageResult, StageResultValidationError>;
```

Rules:

- Triage must include `is_reproducible`, `suspected_area`, `risk`, and `recommended_next_stage`.
- Planning must include `plan_items`, `files_to_inspect_or_change`, `test_strategy`, and `confidence`.
- Implementation must include `changed_files`, `commands_run`, `test_evidence`, and `pr_ready`.
- PR synthesis must include `title`, `body`, `branch_name`, `base_branch`, and `head_sha`.
- CI repair must include `failing_checks`, `diagnosis`, `changed_files`, and `commands_run`.
- Review response must include `addressed_comments`, `remaining_comments`, and `commands_run`.

- [ ] **Step 3: Implement sink adapter**

Add `GithubWorkflowStageResultSink<R>` where `R: GithubIssueWorkflowRepository`. It implements `ironclaw_host_runtime::WorkflowStageResultSink` in the composition crate if adding a direct dependency from the workflow crate to host-runtime would violate boundaries. Prefer implementing the adapter in `crates/ironclaw_reborn_composition/src/github_issue_workflow.rs` to keep the workflow crate independent of host-runtime.

Adapter behavior:

- parse string ids into workflow ids;
- load active stage run;
- verify workflow run id, stage run id, turn run id, stage, schema version, and completion nonce;
- call `validate_stage_result`;
- call `repository.accept_stage_result`;
- return duplicate ack for identical accepted result;
- reject stale attempts and mismatched bindings.

- [ ] **Step 4: Write stage result tests**

Tests:

- `implementation_result_requires_changed_files_and_test_evidence`
- `pr_synthesis_result_requires_branch_and_head_sha`
- `first_valid_stage_result_wins`
- `duplicate_identical_stage_result_replays_ack`
- `mismatched_completion_nonce_is_rejected`
- `stale_stage_attempt_is_rejected`
- `invalid_result_records_validation_failure`

Run: `cargo test -p ironclaw_github_issue_workflow stage_result_contract`.
Expected: all tests pass.

- [ ] **Step 5: Status checkpoint**

Report: strict stage result validation and sink adapter contract pass, no commits made.

## Task 7: Engineered Snapshots And Prompt Pack

**Files:**
- Create: `crates/ironclaw_github_issue_workflow/src/snapshots.rs`
- Create: `crates/ironclaw_github_issue_workflow/src/prompts.rs`
- Create: prompt markdown files under `crates/ironclaw_github_issue_workflow/prompts/github_bug/`
- Test: `crates/ironclaw_github_issue_workflow/tests/prompt_snapshot_contract.rs`

**Interfaces:**
- Consumes: issue observations, workflow state, stage tasks, workspace refs.
- Produces: `StagePromptBundle`, `EngineeredWorkflowSnapshot`, prompt content hash.

- [ ] **Step 1: Implement engineered snapshot model**

Define:

```rust
pub struct EngineeredWorkflowSnapshot {
    pub issue: GithubIssueSnapshot,
    pub workflow: WorkflowStateSnapshot,
    pub repository: RepositorySnapshot,
    pub previous_stage_results: Vec<StageResultSummary>,
    pub workspace: Option<WorkflowWorkspaceSnapshot>,
    pub constraints: StageConstraintSnapshot,
}
```

The snapshot must contain curated fields only. Do not include raw issue body dumps, raw comment dumps, raw host paths, secrets, or backend errors.

- [ ] **Step 2: Implement snapshot hash**

Function:

```rust
pub fn snapshot_hash(snapshot: &EngineeredWorkflowSnapshot) -> Result<String, GithubIssueWorkflowError>;
```

Use canonical JSON serialization plus SHA-256 hex.

- [ ] **Step 3: Implement prompt renderer**

Define:

```rust
pub struct StagePromptBundle {
    pub prompt_ref: String,
    pub prompt_version: String,
    pub content: String,
    pub content_hash: String,
    pub snapshot_hash: String,
}

pub fn render_stage_prompt(
    stage: GithubIssueStage,
    snapshot: &EngineeredWorkflowSnapshot,
) -> Result<StagePromptBundle, GithubIssueWorkflowError>;
```

Prompt text must tell the agent to report completion only through `builtin.workflow_report_stage_result`, with the stage's exact schema requirements.

- [ ] **Step 4: Write prompt files**

Each prompt file should include:

- stage objective;
- allowed tools/fan-out summary;
- context snapshot contract;
- schema requirements;
- success criteria;
- failure/needs-human criteria;
- instruction to avoid GitHub writes and return provider-write intent only.

- [ ] **Step 5: Write prompt/snapshot tests**

Tests:

- `snapshot_hash_is_stable_for_same_snapshot`
- `snapshot_excludes_raw_host_paths`
- `implementation_prompt_names_result_tool_and_schema`
- `planning_prompt_disallows_direct_github_writes`
- `prompt_content_hash_changes_when_prompt_file_changes`

Run: `cargo test -p ironclaw_github_issue_workflow prompt_snapshot_contract`.
Expected: all tests pass.

- [ ] **Step 6: Status checkpoint**

Report: context engineering/prompt contracts pass, no commits made.

## Task 8: App-Side Stage Turn Submitter Over Threads + TurnCoordinator

**Files:**
- Create: `crates/ironclaw_reborn_composition/src/github_issue_workflow.rs`
- Modify: `crates/ironclaw_reborn_composition/src/lib.rs`
- Modify: `crates/ironclaw_reborn_composition/Cargo.toml`
- Test: `crates/ironclaw_reborn_composition/tests/github_issue_workflow_stage_turn.rs`

**Interfaces:**
- Consumes: `SessionThreadService`, `TurnCoordinator`, `SubmitTurnRequest`, `AcceptInboundMessageRequest`.
- Produces: `IronClawStageTurnSubmitter`.

- [ ] **Step 1: Add optional dependencies/features**

In `crates/ironclaw_reborn_composition/Cargo.toml`, add:

```toml
[features]
github-issue-workflow-beta = [
    "dep:ironclaw_github_issue_workflow",
]

[dependencies]
ironclaw_github_issue_workflow = { path = "../ironclaw_github_issue_workflow", optional = true }
```

Later tasks add storage feature deps.

- [ ] **Step 2: Create `github_issue_workflow.rs`**

Define:

```rust
pub(crate) struct IronClawStageTurnSubmitter {
    thread_service: Arc<dyn ironclaw_threads::SessionThreadService>,
    turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator>,
    actor_user_id: ironclaw_host_api::UserId,
    default_agent_id: ironclaw_host_api::AgentId,
}
```

Implement `ironclaw_github_issue_workflow::StageTurnSubmitter` for it.

- [ ] **Step 3: Implement canonical content staging**

Submitter sequence:

1. Derive `ThreadScope` from workflow run tenant/agent/project/owner.
2. Call `SessionThreadService::ensure_thread` with deterministic stage thread id if present.
3. Call `SessionThreadService::replay_accepted_inbound_message` using deterministic `external_event_id`.
4. If replay returns a submitted `turn_run_id`, return the existing run.
5. Otherwise call `accept_inbound_message` with `MessageContent::text(prompt.content)`.
6. Build `SubmitTurnRequest` with `RunProfileRequest::new(request.capability_profile_id)`, deterministic `IdempotencyKey`, and `ProductTurnContext` whose origin is an internal workflow origin represented by the generic existing product context fields.
7. Call `TurnCoordinator::submit_turn`.
8. On accepted, call `mark_message_submitted`.
9. On `TurnError::ThreadBusy`, call `mark_message_rejected_busy` and return a retryable stage submission outcome.

- [ ] **Step 4: Preserve boundaries**

Do not add methods to `ironclaw_product_workflow` for this MVP. This is an internal workflow app submitter over the existing canonical thread/turn contracts.

- [ ] **Step 5: Write stage submitter tests**

Tests:

- `stage_submitter_persists_thread_message_before_turn_submit`
- `stage_submitter_replays_existing_submitted_message`
- `stage_submitter_marks_busy_without_second_turn`
- `stage_submitter_uses_deterministic_idempotency_key`
- `stage_submitter_does_not_use_trusted_trigger_ingress`

Run: `cargo test -p ironclaw_reborn_composition --features github-issue-workflow-beta,test-support github_issue_workflow_stage_turn`.
Expected: all tests pass.

- [ ] **Step 6: Status checkpoint**

Report: stage turns submit through normal thread/turn contracts, no commits made.

## Task 9: Capability Profiles And Read-Only Subagent Guardrails

**Files:**
- Modify: `crates/ironclaw_reborn_composition/src/github_issue_workflow.rs`
- Modify: `crates/ironclaw_reborn_composition/src/local_dev_capability_policy.toml`
- Test: `crates/ironclaw_reborn_composition/tests/github_issue_workflow_capabilities.rs`

**Interfaces:**
- Consumes: existing run-profile/capability surface filtering and `builtin.spawn_subagent`.
- Produces: stage-specific capability profile ids.

- [ ] **Step 1: Define stage profile ids**

Use profile ids:

```text
github-bug-triage-v1
github-bug-planning-v1
github-bug-implementation-v1
github-bug-pr-synthesis-v1
github-bug-ci-repair-v1
github-bug-review-response-v1
```

- [ ] **Step 2: Define capability allowlists**

All stage profiles include `builtin.workflow_report_stage_result`.

Triage/planning:

- `builtin.read_file`
- `builtin.list_dir`
- `builtin.grep`
- `builtin.glob`
- `builtin.spawn_subagent`

Implementation/CI repair:

- `builtin.read_file`
- `builtin.write_file`
- `builtin.apply_patch`
- `builtin.list_dir`
- `builtin.grep`
- `builtin.glob`
- `builtin.shell`
- `builtin.spawn_subagent`

PR synthesis/review response:

- `builtin.read_file`
- `builtin.list_dir`
- `builtin.grep`
- `builtin.glob`
- `builtin.shell`
- `builtin.spawn_subagent`

Do not include `github.create_issue_comment`, `github.comment_issue`, `github.create_pull_request`, `github.reply_pull_request_comment`, `github.merge_pull_request`, or any GitHub write capability in these profiles.

- [ ] **Step 3: Configure subagent flavor policy**

For workflow-issued stage turns, allow model-chosen subagents only for read-only flavors:

- `general`
- `explorer`
- `planner`

Do not allow `coder` inside workflow stage turns in the MVP. Wire the stage-specific `SubagentSpawnCapabilityPort` with a workflow flavor catalog containing only `SpawnSubagentFlavorDescriptor` entries for `general`, `explorer`, and `planner`; `build_spawn_subagent_parameters_schema` must therefore publish an enum that omits `coder`. The corresponding `SubagentDefinitionResolver` must resolve those three flavors to read/search/planning run profiles only.

- [ ] **Step 4: Write capability tests**

Tests:

- `implementation_profile_contains_write_file_patch_shell_and_result_sink`
- `planning_profile_excludes_write_file_patch_shell`
- `all_stage_profiles_exclude_github_write_capabilities`
- `result_sink_is_not_visible_in_non_workflow_default_profile`
- `workflow_stage_profiles_do_not_allow_coder_subagent_flavor`

Run: `cargo test -p ironclaw_reborn_composition --features github-issue-workflow-beta,test-support github_issue_workflow_capabilities`.
Expected: all tests pass. The `workflow_stage_profiles_do_not_allow_coder_subagent_flavor` test should inspect the rendered `builtin.spawn_subagent` parameter schema and assert that `coder` is absent from `properties.subagent_type.enum`.

- [ ] **Step 5: Status checkpoint**

Report: capability guardrails tested, no commits made.

## Task 10: Internal Poller And Discovery Flow

**Files:**
- Create: `crates/ironclaw_github_issue_workflow/src/poller.rs`
- Modify: `crates/ironclaw_github_issue_workflow/src/ports.rs`
- Test: `crates/ironclaw_github_issue_workflow/tests/poller_contract.rs`

**Interfaces:**
- Consumes: workflow repository, GitHub provider port, project access port, workflow config.
- Produces: disabled-by-default internal poller.

- [ ] **Step 1: Implement poller config**

Define:

```rust
pub struct GithubIssueWorkflowPollerConfig {
    pub enabled: bool,
    pub poll_interval: std::time::Duration,
    pub max_repos_per_tick: usize,
    pub max_issues_per_repo_per_tick: usize,
    pub max_runnable_runs_per_tick: usize,
    pub lease_duration: std::time::Duration,
}
```

Defaults: `enabled = false`, `poll_interval = 60s`, `max_repos_per_tick = 20`, `max_issues_per_repo_per_tick = 10`, `max_runnable_runs_per_tick = 10`, `lease_duration = 300s`.

- [ ] **Step 2: Implement discovery tick**

`GithubIssueWorkflowPoller::tick_once()` must:

1. Load enabled workflow configs.
2. Check project access for each config.
3. Search configured repos with query `repo:<owner>/<repo> is:issue state:open label:bug`.
4. Read each issue and comments required for snapshot basics.
5. Normalize to `github.issue.discovered` or `github.issue.changed`.
6. Call `create_or_get_workflow_run`.
7. Call `record_workflow_event`.
8. Claim runnable runs.
9. Call `GithubIssueWorkflowPolicy::tick` for each claimed run.
10. Renew or release/block leases based on outcome.

- [ ] **Step 3: Add backpressure**

The poller must cap work by repository, issue, and runnable-run limits. Rate-limit/provider failures should block only the affected config/run, not the entire process.

- [ ] **Step 4: Write poller tests**

Tests:

- `poller_discovers_bug_issue_and_records_event`
- `poller_dedupes_same_issue_on_second_tick`
- `poller_checks_project_access_before_github_read`
- `poller_applies_per_repo_issue_limit`
- `poller_ticks_runnable_runs_after_discovery`
- `poller_provider_rate_limit_blocks_config_not_process`

Run: `cargo test -p ironclaw_github_issue_workflow poller_contract`.
Expected: all tests pass.

- [ ] **Step 5: Status checkpoint**

Report: internal cron-style poller works with fake GitHub port, no commits made.

## Task 11: GitHub Provider Port Adapter In Composition

**Files:**
- Modify: `crates/ironclaw_reborn_composition/src/github_issue_workflow.rs`
- Test: `crates/ironclaw_reborn_composition/tests/github_issue_workflow_provider.rs`

**Interfaces:**
- Consumes: existing GitHub first-party extension capabilities.
- Produces: `IronClawGithubIssueWorkflowPort`.

- [ ] **Step 1: Map provider port to existing capability names**

Use these existing GitHub capabilities:

- read/search: `github.search_issues`, `github.search_issues_pull_requests`, `github.get_issue`, `github.list_issue_comments`;
- claim/comment writes: `github.create_issue_comment` or `github.comment_issue`;
- PR lifecycle: `github.create_pull_request`, `github.get_pull_request`, `github.get_pull_request_files`, `github.list_pull_request_comments`, `github.get_pull_request_reviews`, `github.reply_pull_request_comment`;
- auth/account: `github.get_authenticated_user`;
- later webhook normalization: GitHub extension enriched payload support in `tools-src/github/src/lib.rs`.

- [ ] **Step 2: Implement workflow provider adapter**

The adapter should invoke GitHub capabilities through the same host-runtime mediated capability path used by other first-party extension calls. It must not construct raw GitHub HTTP clients or bypass network/secret policy.

- [ ] **Step 3: Normalize responses into workflow observations**

Convert capability JSON/string responses into typed `GithubIssueObservation`, `GithubCommentObservation`, `GithubPullRequestLifecycleObservation`, and provider refs. If provider output lacks node ids, use stable owner/repo/number refs and mark `node_id = None`.

- [ ] **Step 4: Write provider adapter tests**

Use fake capability dispatch, not live GitHub.

Tests:

- `search_open_bug_issues_invokes_search_issues_with_expected_query`
- `create_claim_comment_invokes_comment_issue_with_marker_body`
- `create_draft_pr_invokes_create_pull_request_with_draft_true`
- `provider_adapter_redacts_backend_error`
- `provider_adapter_uses_configured_account_ref`

Run: `cargo test -p ironclaw_reborn_composition --features github-issue-workflow-beta,test-support github_issue_workflow_provider`.
Expected: all tests pass.

- [ ] **Step 5: Status checkpoint**

Report: GitHub provider adapter works against fake capability dispatch, no commits made.

## Task 12: Durable Storage Adapter With libSQL/PostgreSQL Parity

**Files:**
- Modify: root `Cargo.toml`
- Create: `crates/ironclaw_github_issue_workflow_storage/Cargo.toml`
- Create: `crates/ironclaw_github_issue_workflow_storage/src/lib.rs`
- Create: `crates/ironclaw_github_issue_workflow_storage/src/filesystem_repository.rs`
- Create: `crates/ironclaw_github_issue_workflow_storage/src/filesystem_repository/path.rs`
- Test: `crates/ironclaw_github_issue_workflow_storage/tests/durable_repository_contract.rs`
- Test: `crates/ironclaw_github_issue_workflow_storage/tests/support/mod.rs`

**Interfaces:**
- Consumes: `GithubIssueWorkflowRepository`.
- Produces: `RebornFilesystemGithubIssueWorkflowRepository`, `RebornLibSqlGithubIssueWorkflowRepository`, `RebornPostgresGithubIssueWorkflowRepository`.

- [ ] **Step 1: Add storage crate to workspace**

Add `"crates/ironclaw_github_issue_workflow_storage"` to root `Cargo.toml`.

- [ ] **Step 2: Create storage manifest**

Mirror `ironclaw_product_workflow_storage`:

```toml
[package]
name = "ironclaw_github_issue_workflow_storage"
version = "0.1.0"
edition = "2024"
rust-version = "1.92"
description = "Durable storage adapters for the IronClaw GitHub issue workflow"
authors = ["NEAR AI <support@near.ai>"]
license = "MIT OR Apache-2.0"
homepage = "https://github.com/nearai/ironclaw"
repository = "https://github.com/nearai/ironclaw"
publish = false

[features]
default = []
libsql = ["ironclaw_filesystem/libsql"]
postgres = ["ironclaw_filesystem/postgres"]

[dependencies]
async-trait = "0.1"
chrono = { version = "0.4", features = ["serde"] }
ironclaw_filesystem = { path = "../ironclaw_filesystem", version = "0.1.0" }
ironclaw_github_issue_workflow = { path = "../ironclaw_github_issue_workflow", version = "0.1.0" }
ironclaw_host_api = { path = "../ironclaw_host_api", version = "0.1.0" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
tracing = "0.1"

[dev-dependencies]
deadpool-postgres = "0.14"
ironclaw_filesystem = { path = "../ironclaw_filesystem", features = ["libsql", "postgres"] }
libsql = { version = "0.6", default-features = false, features = ["core", "replication", "remote", "tls"] }
tempfile = "3"
tokio = { version = "1", features = ["macros", "rt", "sync", "time"] }
tokio-postgres = "0.7"
```

- [ ] **Step 3: Implement filesystem-backed records**

Use `ScopedFilesystem` record APIs and CAS semantics. Store records under a workflow-owned prefix, for example:

```text
/github_issue_workflow/configs/<tenant>/<project>.json
/github_issue_workflow/runs/<tenant>/<workflow_run_id>.json
/github_issue_workflow/run_keys/<tenant>/<workflow_run_key_hash>.json
/github_issue_workflow/events/<workflow_run_id>/<sequence>.json
/github_issue_workflow/event_keys/<workflow_run_id>/<idempotency_key_hash>.json
/github_issue_workflow/stages/<workflow_run_id>/<stage_run_id>.json
/github_issue_workflow/provider_actions/<workflow_run_id>/<action_key_hash>.json
/github_issue_workflow/provider_bindings/<provider_ref_hash>.json
```

- [ ] **Step 4: Preserve atomic method semantics**

All repository methods must preserve the same outcomes as in-memory. Use filesystem CAS loops where necessary. Do not expose raw filesystem errors; map to `GithubIssueWorkflowError::StorageUnavailable` with sanitized reason.

- [ ] **Step 5: Write durable repository parity tests**

Run the same contract suite against:

- in-memory;
- libSQL filesystem backend;
- PostgreSQL filesystem backend.

Tests:

- `durable_create_or_get_workflow_run_is_idempotent`
- `durable_event_recording_is_idempotent`
- `durable_lease_claim_excludes_unexpired_lease`
- `durable_transition_rejects_stale_version`
- `durable_stage_uniqueness_survives_reload`
- `durable_provider_action_dedupes_after_reload`
- `durable_provider_binding_routes_after_reload`

Run:

```bash
cargo test -p ironclaw_github_issue_workflow_storage --features libsql durable_repository_contract
cargo test -p ironclaw_github_issue_workflow_storage --features postgres durable_repository_contract
```

Expected: libSQL and PostgreSQL tests pass in environments with their backends available.

- [ ] **Step 6: Status checkpoint**

Report: durable parity contracts pass or name the unavailable backend explicitly, no commits made.

## Task 13: Runtime Config And Composition Wiring

**Files:**
- Modify: `crates/ironclaw_reborn_config/src/config_file.rs`
- Modify: `crates/ironclaw_reborn_config/src/lib.rs`
- Modify: `crates/ironclaw_reborn_composition/src/runtime_input.rs`
- Modify: `crates/ironclaw_reborn_composition/src/runtime.rs`
- Modify: `crates/ironclaw_reborn_composition/src/factory.rs`
- Modify: `crates/ironclaw_reborn_composition/Cargo.toml`
- Test: `crates/ironclaw_reborn_config/tests/config_file.rs` or existing inline config tests in `config_file.rs`
- Test: `crates/ironclaw_reborn_composition/tests/github_issue_workflow_runtime.rs`

**Interfaces:**
- Consumes: workflow app/storage crates and host-runtime result sink.
- Produces: disabled-by-default runtime worker, readiness bit, config section.

- [ ] **Step 1: Add `[github_issue_workflow]` config section**

In `ironclaw_reborn_config::RebornConfigFile`, add:

```rust
pub github_issue_workflow: Option<GithubIssueWorkflowConfigSection>,
```

Define:

```rust
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GithubIssueWorkflowConfigSection {
    pub enabled: Option<bool>,
    pub poll_interval_secs: Option<u64>,
    pub max_repos_per_tick: Option<usize>,
    pub max_issues_per_repo_per_tick: Option<usize>,
    pub max_runnable_runs_per_tick: Option<usize>,
    pub lease_duration_secs: Option<u64>,
}
```

Parsing only. Do not perform runtime side effects in `ironclaw_reborn_config`.

- [ ] **Step 2: Add config parser tests**

Tests:

- `github_issue_workflow_full_section_parses`
- `github_issue_workflow_absent_section_yields_none`
- `github_issue_workflow_rejects_unknown_key`
- `github_issue_workflow_section_rejects_inline_secret_strings_if_string_fields_are_added`

Run: `cargo test -p ironclaw_reborn_config github_issue_workflow`.
Expected: all tests pass.

- [ ] **Step 3: Add runtime settings**

In `runtime_input.rs`, add:

```rust
#[derive(Debug, Clone)]
pub struct GithubIssueWorkflowSettings {
    pub enabled: bool,
    pub poll_interval: Duration,
    pub max_repos_per_tick: usize,
    pub max_issues_per_repo_per_tick: usize,
    pub max_runnable_runs_per_tick: usize,
    pub lease_duration: Duration,
}
```

Add `GithubIssueWorkflowSettings::disabled()` and `GithubIssueWorkflowSettings::enabled_for_tests()`.

- [ ] **Step 4: Wire storage in factory**

In `factory.rs`, add workflow repository fields to local runtime services and production graph:

- in-memory when durable storage is not enabled;
- `RebornLibSqlGithubIssueWorkflowRepository` under `libsql`;
- `RebornPostgresGithubIssueWorkflowRepository` under `postgres`.

Do not expose lower substrate handles publicly. Follow the existing trigger/product workflow storage patterns.

- [ ] **Step 5: Wire host-runtime result sink**

When workflow is enabled, composition must call `builtin_first_party_handlers_with_workflow_stage_result_sink(...)` and pass the workflow sink adapter. When workflow is disabled, keep the current handler construction path unchanged.

- [ ] **Step 6: Start/stop poller in runtime**

In `runtime.rs`, add:

- `github_issue_workflow_handle: Option<GithubIssueWorkflowRuntimeHandle>` to `RebornRuntime`;
- cancellation/shutdown behavior mirroring `trigger_poller_handle`;
- readiness bit `workers.github_issue_workflow`;
- tests that disabled default does not start it.

- [ ] **Step 7: Production fail-closed rules**

Production enablement must fail startup if:

- workflow is enabled and no durable storage backend is available;
- workflow is enabled and no GitHub provider account reference is configured;
- workflow is enabled and project-scoped configs lack `project_id`;
- workflow is enabled but project access checker cannot be wired.

Local-dev may start with in-memory config only when explicitly enabled for tests.

- [ ] **Step 8: Runtime wiring tests**

Tests:

- `runtime_disables_github_issue_workflow_by_default`
- `runtime_starts_github_issue_workflow_when_enabled`
- `runtime_shutdown_cancels_github_issue_workflow_poller`
- `runtime_enabled_workflow_registers_result_sink_handler`
- `production_enabled_workflow_requires_durable_storage`
- `production_enabled_workflow_requires_project_access_checker`

Run: `cargo test -p ironclaw_reborn_composition --features github-issue-workflow-beta,test-support github_issue_workflow_runtime`.
Expected: all tests pass.

- [ ] **Step 9: Status checkpoint**

Report: binary/runtime wiring works and remains disabled by default, no commits made.

## Task 14: Workspace Preparation And Implementation Stage Loop

**Files:**
- Modify: `crates/ironclaw_github_issue_workflow/src/ports.rs`
- Modify: `crates/ironclaw_github_issue_workflow/src/policy.rs`
- Modify: `crates/ironclaw_reborn_composition/src/github_issue_workflow.rs`
- Test: `crates/ironclaw_github_issue_workflow/tests/workspace_stage_contract.rs`

**Interfaces:**
- Consumes: existing IronClaw filesystem/workspace/host-runtime boundaries.
- Produces: workspace refs and mount refs only, not raw paths.

- [ ] **Step 1: Implement workspace session model**

Define:

```rust
pub struct GithubIssueWorkspaceSession {
    pub workspace_session_id: GithubIssueWorkspaceSessionId,
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub repository: GithubRepositorySelector,
    pub base_branch: String,
    pub base_sha: Option<String>,
    pub working_branch: String,
    pub current_head_sha: Option<String>,
    pub workspace_ref: WorkflowWorkspaceRef,
    pub mount_ref: WorkflowWorkspaceMountRef,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
```

- [ ] **Step 2: Implement workspace manager adapter**

The composition adapter must prepare an isolated workspace for the issue. It may use existing local checkout/materialization mechanisms, but the workflow repository stores only `workspace_ref` and `mount_ref`.

- [ ] **Step 3: Policy integration**

Planning completion prepares workspace before implementation. Implementation stage prompts receive:

- workspace mount alias;
- base branch/head summary;
- allowed command/test policy;
- no raw host path.

- [ ] **Step 4: Workspace tests**

Tests:

- `planning_completion_prepares_workspace_once`
- `workspace_ref_not_raw_host_path`
- `implementation_stage_receives_mount_ref`
- `workspace_prepare_failure_blocks_run_retryably`

Run: `cargo test -p ironclaw_github_issue_workflow workspace_stage_contract`.
Expected: all tests pass.

- [ ] **Step 5: Status checkpoint**

Report: implementation stage can run against isolated workspace refs, no commits made.

## Task 15: Draft PR Provider Action And Lifecycle Refresh

**Files:**
- Modify: `crates/ironclaw_github_issue_workflow/src/policy.rs`
- Modify: `crates/ironclaw_github_issue_workflow/src/provider_actions.rs`
- Modify: `crates/ironclaw_github_issue_workflow/src/poller.rs`
- Test: `crates/ironclaw_github_issue_workflow/tests/pr_lifecycle_contract.rs`

**Interfaces:**
- Consumes: implementation and PR synthesis stage results.
- Produces: draft PR creation/reconciliation and lifecycle refresh events.

- [ ] **Step 1: PR synthesis transition**

When implementation completes with `pr_ready = true`, start `pr_synthesis`. When PR synthesis reports valid branch/head/title/body, create or replay provider action `create_or_update_pr`.

- [ ] **Step 2: Draft PR action**

Provider action must:

- create draft PR through provider port;
- include stable PR marker in body;
- record `primary_pr` provider binding;
- reconcile by head branch plus marker.

- [ ] **Step 3: Lifecycle refresh**

Poller active-run refresh must read PR state and checks:

- `github.pr.updated`
- `github.checks.failed`
- `github.checks.succeeded`
- `github.review_comment.created`

It records workflow events, then policy starts `ci_repair` or `review_response` stages when needed.

- [ ] **Step 4: PR lifecycle tests**

Tests:

- `pr_synthesis_creates_draft_pr_once`
- `draft_pr_ambiguous_write_reconciles_by_branch_and_marker`
- `failed_checks_start_ci_repair_stage`
- `review_comment_starts_review_response_stage`
- `merged_pr_completes_workflow`
- `closed_issue_cancels_active_workflow`

Run: `cargo test -p ironclaw_github_issue_workflow pr_lifecycle_contract`.
Expected: all tests pass.

- [ ] **Step 5: Status checkpoint**

Report: draft PR and lifecycle refresh policy pass tests, no commits made.

## Task 16: Observability, Redaction, And Smoke Test

**Files:**
- Modify: `crates/ironclaw_github_issue_workflow/src/policy.rs`
- Modify: `crates/ironclaw_github_issue_workflow/src/poller.rs`
- Modify: `crates/ironclaw_reborn_composition/src/github_issue_workflow.rs`
- Modify: `docs/superpowers/specs/2026-06-15-github-bug-orchestration-design.md` if implementation details differ.
- Test: `crates/ironclaw_reborn_composition/tests/github_issue_workflow_smoke.rs`

**Interfaces:**
- Consumes: workflow events/actions/stage runs.
- Produces: redacted lifecycle telemetry and smoke confidence.

- [ ] **Step 1: Emit redacted lifecycle events**

Use existing event/audit patterns where composition already exposes event sinks. Events should include:

- workflow run created/claimed/blocked/succeeded/failed;
- stage run started/succeeded/failed;
- provider action pending/succeeded/needs reconciliation/failed;
- poller tick summary.

Do not include raw issue bodies, raw comments, model output, tool arguments, secrets, raw provider errors, or host paths.

- [ ] **Step 2: Add smoke test with fake GitHub and fake model completion**

End-to-end fake flow:

```text
open bug issue
  -> poller discovers issue
  -> claim comment action succeeds
  -> triage result reported
  -> planning result reported
  -> workspace prepared
  -> implementation result reported
  -> PR synthesis result reported
  -> draft PR action succeeds
  -> workflow status active/pr_open
```

- [ ] **Step 3: Add failure smoke test**

Fake invalid implementation result:

```text
implementation stage reports pr_ready without commands_run
  -> sealed result rejected
  -> stage validation failure recorded
  -> workflow remains retryable/blocked according to policy
```

- [ ] **Step 4: Run targeted gates**

Run:

```bash
cargo test -p ironclaw_github_issue_workflow
cargo test -p ironclaw_host_runtime --test workflow_result_tool_contract
cargo test -p ironclaw_reborn_config github_issue_workflow
cargo test -p ironclaw_reborn_composition --features github-issue-workflow-beta,test-support github_issue_workflow
cargo test -p ironclaw_architecture
```

Expected: all available targeted gates pass. If a backend-specific durable test cannot run locally, record the exact missing service.

- [ ] **Step 5: Status checkpoint**

Report: smoke flow and targeted gates complete, no commits made.

## Task 17: Future Webhook Slot-In Point

**Files:**
- Modify: `crates/ironclaw_github_issue_workflow/src/workflow_events.rs`
- Modify: `crates/ironclaw_github_issue_workflow/src/ports.rs`
- Test: `crates/ironclaw_github_issue_workflow/tests/webhook_readiness_contract.rs`

**Interfaces:**
- Consumes: existing workflow event envelope.
- Produces: webhook normalizer function only; no HTTP listener.

- [ ] **Step 1: Add normalizer API**

Define:

```rust
pub fn normalize_github_webhook_event(
    input: NormalizeGithubWebhookEventInput,
) -> Result<Vec<WorkflowEventEnvelope<serde_json::Value>>, GithubIssueWorkflowError>;
```

This function accepts already authenticated/enriched webhook payload metadata and maps it to the same workflow events the poller records.

- [ ] **Step 2: Keep listener out of scope**

Do not add an Axum route, public webhook endpoint, signing-secret handling, or route mount. Those belong to a later ingress plan. This task proves that when webhook ingress exists, it can feed the same event store without changing policy.

- [ ] **Step 3: Webhook readiness tests**

Tests:

- `issues_webhook_normalizes_to_issue_changed_event`
- `issue_comment_webhook_on_pr_routes_to_pr_comment_event`
- `pull_request_review_comment_webhook_routes_by_provider_binding`
- `duplicate_webhook_delivery_reuses_source_delivery_id`
- `self_authored_webhook_echo_is_suppressed_when_binding_matches`

Run: `cargo test -p ironclaw_github_issue_workflow webhook_readiness_contract`.
Expected: all tests pass.

- [ ] **Step 4: Status checkpoint**

Report: webhook slot-in normalizer exists without changing MVP ingress, no commits made.

## Execution Order And Review Gates

Recommended implementation order:

1. Tasks 1-4: pure workflow app domain/repository/policy with fake ports.
2. Tasks 5-6: sealed result primitive and validation adapter.
3. Tasks 7-9: prompt/context and stage turn/capability integration.
4. Tasks 10-11: poller and GitHub provider port adapter.
5. Tasks 12-13: durable storage and runtime wiring.
6. Tasks 14-16: workspace, PR lifecycle, observability, smoke.
7. Task 17: webhook readiness normalizer.

Review gates:

- After Task 4: validate that orchestration policy is app-owned and not leaking into IronClaw core.
- After Task 6: validate the sealed result contract before any prompt work depends on it.
- After Task 9: validate agent/subagent/capability boundaries.
- After Task 13: validate what actually changed inside the Reborn binary.
- After Task 16: validate MVP behavior against the design success criteria.

## Verification Checklist

- `rg "TrustedInboundTurnRequest|TrustedTriggerSubmitRequest|trusted_submit" crates/ironclaw_github_issue_workflow crates/ironclaw_reborn_composition/src/github_issue_workflow.rs` shows no workflow usage except negative tests/docs.
- `rg "github.create_pull_request|github.comment_issue|github.create_issue_comment|github.reply_pull_request_comment" crates/ironclaw_github_issue_workflow/prompts` confirms prompts forbid direct model writes and route writes through provider actions.
- `rg "workflow_report_stage_result" crates/ironclaw_reborn_composition` confirms the result capability is only in workflow stage profiles.
- `cargo test -p ironclaw_github_issue_workflow` passes.
- `cargo test -p ironclaw_host_runtime --test workflow_result_tool_contract` passes.
- `cargo test -p ironclaw_github_issue_workflow_storage --features libsql durable_repository_contract` passes.
- `cargo test -p ironclaw_github_issue_workflow_storage --features postgres durable_repository_contract` passes where PostgreSQL test services are available.
- `cargo test -p ironclaw_reborn_composition --features github-issue-workflow-beta,test-support github_issue_workflow` passes.
- `cargo test -p ironclaw_architecture` passes after new crate dependency edges are added.

## Self-Review Notes

- The design's only actual platform primitive is covered by Task 5.
- The workflow application boundary is covered by Tasks 1-4 and 7-17.
- Project scoping is covered by Tasks 1, 10, and 13.
- Provider writes and echo suppression are covered by Tasks 3, 11, and 15.
- Idempotency and leases are covered by Tasks 2, 4, and 12.
- Agent delegation is handled through existing IronClaw subagents and constrained by Task 9; no subagent runtime is ported.
- Webhook slot-in is covered by Task 17 without changing the MVP cron ingress.
- No task contains commit steps, in accordance with Ben's instruction.
