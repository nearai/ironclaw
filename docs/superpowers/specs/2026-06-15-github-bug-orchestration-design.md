# GitHub Bug-Fix Workflow Application MVP - Design

**Date:** 2026-06-15
**Status:** Draft revised after 2026-06-22 IronClaw/Stevie pull review
**Target architecture:** IronClaw Reborn as the agent execution platform
**MVP ingress:** Internal cron-style poller only
**Primary goal:** Automate GitHub issues labeled `bug` with a hardened
workflow application built on IronClaw primitives.

## 1. Purpose

Build a first-party GitHub bug-fix workflow application that discovers GitHub
issues tagged `bug`, claims one safely, drives IronClaw agents through a
structured bug-fix lifecycle, and opens a draft pull request.

The goal is not "run an agent every few minutes." It is also not "put all of
GitHub bug-fixing into IronClaw core." The goal is to prove that IronClaw has
enough platform primitives for a workflow application to achieve unattended
coding reliability:

- durable workflow runs;
- ordered workflow events;
- workflow policy/state machine orchestration;
- driver-style workflow decisions where topology should be model-led;
- context-engineered workflow snapshots rather than raw state dumps;
- stage-specific prompts;
- structured model results;
- idempotent provider actions;
- bounded agent fan-out;
- workspace isolation;
- lifecycle re-entry for PR, CI, and review events.

The workflow application may run in-process as first-party IronClaw crates for
the MVP, or later as a separate service that calls IronClaw through stable
platform APIs. That deployment choice must not weaken the contracts. The
application still owns strict schema validation, idempotency, provider
reconciliation, fan-out limits, and recovery semantics.

For MVP, ingress is deliberately boring:

```text
internal fixed-interval poller
  -> observe GitHub issue/PR/check/review state
  -> normalize observations into workflow events
  -> tick runnable workflow runs
```

GitHub webhooks, benchmark-result webhooks, and review-comment push delivery
are deferred. The design keeps those future ingress paths easy by making workflow events,
not polling snapshots, the boundary into orchestration.

## 2. Audit-Driven Corrections

This revision incorporates four independent audit passes. The important
corrections are:

- Stage runs submit normal scoped IronClaw turns. They do not use trusted
  trigger ingress.
- Cron/polling is only an ingress source. It does not own orchestration.
- GitHub bug-fix lifecycle policy is application-level workflow logic, not
  agent-loop, turn-coordinator, trigger, or GitHub-extension core logic.
- IronClaw core changes should be limited to reusable platform primitives:
  scoped work submission, structured result capture, status/correlation
  surfaces, capability profiles, and workspace/run metadata.
- Workflow run status is split from workflow mode/stage.
- Provider writes are performed by workflow provider actions, not directly by stage
  agents.
- Provider bindings and echo suppression are first-class requirements.
- Leases, cursors, idempotency, and reconciliation are explicit contracts.
- Workspace records expose scoped workspace/mount refs, not raw host paths.
- `workflow.report_stage_result` is a sealed, stage-bound result sink.
- "Application on top of IronClaw" does not mean a thin cron script. The
  workflow application must retain production-grade output verification,
  replay/recovery, and bounded delegation behavior.
- IronClaw Projects are now first-class Reborn entities. GitHub bug automation
  configuration should be project-scoped when a project exists, and repository
  bindings should live in project/workflow configuration rather than loose
  process config.
- IronClaw one-shot trigger support makes future user-configurable scans easier,
  but it does not move GitHub bug lifecycle policy into `ironclaw_triggers`.
- Per-capability permission overrides and external-write effects strengthen the
  "agents propose, provider actions write" boundary. Workflow-issued stage runs
  should receive read/write coding authority separately from GitHub write
  authority.
- IronClaw's output-aware progress detection and terminal honesty improve
  stage-run classification, but sealed structured results remain the workflow
  source of truth.
- Stevie's latest orchestrator work reinforces a driver-led, recipe-fenced
  pattern: the application owns deterministic rails, while short driver
  episodes may decide plan deltas, routing, and fan-out against strict schemas.
  The IronClaw workflow should preserve that option instead of hard-coding every
  topology into the outer state machine.
- The MVP uses only GitHub capabilities that exist today: issue search/read,
  comments, PR create/read, branch/file capabilities, GitHub Actions run
  listings, combined status, PR review comments, authenticated-user lookup, and
  webhook payload handling.

The resulting design is narrower than "build a generic orchestration kernel," but it keeps the
IronClaw-native reliability model.

## 3. Existing IronClaw Platform Foundation

IronClaw already has several platform surfaces this workflow application should
reuse:

- Reborn turns and run profiles provide the normal path for agent execution.
- Product workflow owns user/product-facing turn acceptance, canonical content
  persistence, idempotency, busy/deferred handling, and safe acknowledgements.
- First-party GitHub extension capabilities cover issue search/read/comment,
  PR create/read, branch/file capabilities, GitHub Actions run listings,
  combined status, PR review comments, authenticated-user lookup, repository
  search, code search, issue/PR search, and webhook payload handling.
- First-party coding capabilities cover read, write, list, glob, grep, patch,
  and shell/test execution through runtime policy.
- Filesystem and host-runtime crates own path containment, mounts, sandbox
  process setup, and runtime execution.
- `ironclaw_projects` now owns durable Reborn project records, membership, live
  access checks, and project metadata. Workflow configuration should bind to
  this layer where possible.
- Per-tool capability permission overrides now exist in approvals/runtime
  policy, including explicit disabled/ask states in addition to durable
  always-allow grants.
- Output-aware progress detection now compares normalized capability output
  digests, reducing false progress/no-progress conclusions in long-running
  stages.
- `ironclaw_triggers` owns user-facing scheduled trigger records and trusted
  scheduled-trigger ingress, including first-class `Once` schedules.
- Events/projections/outbound surfaces already define the shape for redacted
  observability and future UI exposure.

The missing layer is not a new agent runtime. It is a GitHub issue workflow
application that consumes the platform:

- workflow run/workflow event storage;
- workflow policy tick/lease semantics;
- workflow provider action records;
- issue/PR provider-binding routing;
- stage turn submission and result capture;
- per-workflow-run workspace ownership.

## 4. Boundary Decisions

### 4.1 Platform/Application Boundary

The GitHub bug-fix system should be treated as an application on top of
IronClaw's agent platform.

IronClaw should provide reusable platform primitives:

- submit scoped agent work with idempotency and correlation metadata;
- execute turns, tools, checkpoints, gates, approvals, and subagents;
- expose run status and lifecycle events;
- enforce capability profiles and runtime/workspace boundaries;
- provide provider capability surfaces such as GitHub;
- support a sealed structured-result path for workflow-issued turns.

The workflow application should own GitHub-specific orchestration:

- candidate issue selection;
- one active workflow run per GitHub issue;
- project-scoped repository configuration and automation settings;
- workflow event ingestion and ordering;
- workflow policy/state-machine transitions;
- stage prompts, result schemas, and validation;
- GitHub claim/PR/review/CI policy;
- provider action idempotency and reconciliation;
- provider bindings and self-echo suppression.

This keeps IronClaw core general while still allowing the GitHub bug-fix app to
be as rigorous as a bespoke orchestration system. Any required IronClaw changes
should improve general platform primitives rather than encode GitHub bug-fix
lifecycle rules in core.

### 4.2 Cron Boundary

The MVP uses an internal composition-owned workflow poller with a fixed
interval, backoff, and per-repo limits.

This is intentionally not a user-created `ironclaw_triggers` schedule record.
It is host-owned maintenance work for the GitHub issue workflow, similar to a
service background worker.

The internal poller must not duplicate general trigger semantics:

- no user-authored cron expression parser;
- no user-facing trigger management API;
- no trusted trigger conversation submission;
- no notification/delivery policy;
- no trigger source provider abstraction.

If this later becomes user-configurable scheduled automation, that should be a
separate integration with `ironclaw_triggers`. At that point, triggers own
schedule records and due-fire identity; this workflow still owns GitHub issue
workflow runs, workflow events, workflow policies, and provider actions.

The 2026-06-22 pull added first-class `TriggerSchedule::Once`, so one-shot
automation is no longer modeled as a cron workaround. That improves future
"run this scan once" product UX, but it does not change the MVP boundary:
trigger fires may request a scan, while the GitHub issue workflow remains the
owner of issue lifecycle state.

### 4.3 Trusted Ingress Boundary

Only trigger-owned code may mint trusted trigger inbound requests.

This workflow must not construct or call:

- `TrustedInboundTurnRequest`;
- `TrustedTriggerSubmitRequest`;
- trigger trusted submitter constructors;
- trigger-specific synthetic inbound seams.

Stage runs submit normal scoped workflow turns through a product-workflow or
turn-coordinator facade that persists canonical thread content and enforces
normal admission, auth, run-profile, capability, and idempotency rules.

### 4.4 Provider Write Boundary

Agents do not directly perform GitHub writes for the workflow.

Stage agents produce structured intent. The workflow policy/workflow performs provider
writes through provider action records:

- claim comment;
- issue comment;
- branch creation/push;
- draft PR creation;
- review reply.

Every provider write must go through one mediated authority path wired by
composition:

- scoped GitHub credential/account selection;
- host-runtime/network egress policy;
- secret lease handling;
- approval/runtime policy where required;
- provider action record idempotency;
- provider action reconciliation.

### 4.5 Workspace Boundary

Workflow domain records may store scoped workspace refs and mount aliases. They
must not expose raw host paths to product surfaces or model-visible context.

Raw paths, clone directories, sandbox implementation details, and host process
handles belong inside the workspace manager / host-runtime layer.

### 4.6 Storage Boundary

The workflow introduces new persistence. It must follow IronClaw's normal
storage rule:

- define repository traits first;
- implement in-memory behavior for tests;
- implement libSQL and PostgreSQL parity before production use;
- test unique keys, idempotency, CAS/lease behavior, tenant scoping, and
  failure hydration in both durable backends.

## 5. High-Level Architecture

```text
GitHub bug-fix workflow application
  -> internal workflow poller
      -> GithubIssueIngressNormalizer
          -> WorkflowEventEnvelope
              -> WorkflowRunRepository.record_workflow_event()
              -> WorkflowRunScheduler.claim_runnable_workflow_runs()
                  -> GithubIssueWorkflowPolicy.tick()
                      -> durable workflow step
                          -> provider action runner for provider writes
                          -> stage runner for normal IronClaw turns
                          -> workspace manager for scoped workspaces
```

Later ingress paths fit into the same middle:

```text
GitHub webhook
  -> GithubIssueIngressNormalizer
      -> WorkflowEventEnvelope
          -> same workflow event store
          -> same workflow policy tick

benchmark result
  -> BenchmarkIngressNormalizer
      -> WorkflowEventEnvelope
          -> same workflow event store
          -> same workflow policy tick
```

The workflow policy never cares whether a workflow event came from polling or a webhook. It only
sees ordered, validated, idempotent workflow events. Likewise, the IronClaw
turn layer never needs to know the GitHub bug lifecycle; it receives scoped
stage work and returns status/results through platform contracts.

## 6. Crate And Module Shape

Recommended in-tree MVP crate layout:

```text
crates/ironclaw_github_issue_workflow/
  src/lib.rs
  src/domain.rs
  src/workflow_events.rs
  src/policy.rs
  src/steps.rs
  src/provider_actions.rs
  src/stages.rs
  src/prompts.rs
  src/poller.rs
  src/ports.rs
  src/workspace.rs

crates/ironclaw_github_issue_workflow_storage/
  src/lib.rs
  src/libsql.rs
  src/postgres.rs

crates/ironclaw_reborn_composition/src/github_issue_workflow.rs
```

This layout keeps the workflow application close to IronClaw while the platform
contracts mature. If the application is later externalized, the same domain,
policy, storage, and provider-action modules should move behind an API boundary
rather than being replaced by an ad hoc harness.

Ownership:

- `ironclaw_github_issue_workflow` is the first-party workflow application
  crate. It owns domain contracts, workflow policy,
  provider action semantics, workflow event normalization, and workflow-owned ports.
- `ironclaw_github_issue_workflow_storage` owns durable libSQL/PostgreSQL
  adapters.
- `ironclaw_reborn_composition` wires repositories, provider ports, turn
  submitters, workspace manager, event sinks, and runtime settings.
- IronClaw core/platform crates own only the generic primitives needed by this
  and future workflow applications.
- `ironclaw_projects` owns project membership and project metadata. The workflow
  application may store GitHub automation configuration in project-scoped
  workflow config or in a project metadata sub-object, but it must not bypass
  live project access checks.

Do not put this policy into:

- `ironclaw_triggers`: schedule records are not GitHub issue policy;
- GitHub WASM extension: provider capabilities are not orchestration;
- `ironclaw_reborn_composition`: composition should not become the state
  machine;
- `ironclaw_turns`: turns must not receive raw prompts/tool inputs/host paths
  from workflow policy or GitHub-specific lifecycle rules.

## 6.1 Project-Scoped Configuration

The pulled IronClaw main now has a first-class Reborn Project layer. The GitHub
bug-fix workflow should treat project scope as the normal product boundary.

Recommended MVP configuration:

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

Rules:

- every live scan is authorized against the project before it reads or writes;
- repository selectors are host/project configuration, never issue text;
- workflow runs carry `project_id` for turn scope, workspace scope,
  observability, and WebUI routing;
- local-dev may still support a no-project fallback, but production automation
  should be project-scoped.

## 7. Canonical Data Model

### 7.1 Workflow Run

The workflow run is the durable unit of work for one GitHub issue.

```rust
pub struct GithubIssueWorkflowRun {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub workflow_run_key: GithubIssueWorkflowRunKey,

    pub tenant_id: TenantId,
    pub creator_user_id: UserId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,

    pub issue_ref: GithubIssueRef,
    pub workflow_policy_key: String,
    pub workflow_policy_version: String,

    pub status: GithubIssueWorkflowRunStatus,
    pub workflow_state: GithubIssueWorkflowState,

    pub event_cursor: i64,
    pub workflow_run_version: i64,
    pub lease_owner: Option<WorkflowWorkerId>,
    pub lease_expires_at: Option<Timestamp>,
    pub last_heartbeat_at: Option<Timestamp>,
    pub claim_count: u32,

    pub active_stage_run_id: Option<GithubIssueStageRunId>,
    pub workspace_session_id: Option<GithubIssueWorkspaceSessionId>,

    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}
```

`workflow_run_key` is globally unique per tenant:

```text
github-issue:v1:<owner>/<repo>#<issue_number>
```

`project_id` is optional only to preserve local-dev and transitional wiring.
For product use, the workflow run should be project-scoped and should fail
closed when the project no longer grants the automation owner access.

### 7.2 Workflow Run Status

Workflow run status is intentionally small:

```text
active
blocked
succeeded
failed
cancelled
```

Workflow policy stage is not workflow run status. Workflow policy stage belongs in
`GithubIssueWorkflowState`.

### 7.3 Workflow State

```rust
pub struct GithubIssueWorkflowState {
    pub mode: GithubIssueWorkflowMode,
    pub active_block: Option<GithubIssueBlockState>,
    pub plan: Vec<GithubIssuePlanItem>,
    pub primary_pr: Option<GithubPullRequestRef>,
    pub claim_comment: Option<GithubCommentRef>,
    pub current_workspace_ref: Option<WorkflowWorkspaceRef>,
    pub last_provider_watermarks: GithubProviderWatermarks,
}
```

Workflow modes:

```text
new
claimed
triage
planning
implementation
pr_synthesis
pr_open
ci_repair
review_response
done
```

Block states are structured:

```text
waiting_approval
waiting_auth
blocked_human
recovery_required
rate_limited
terminal_failed
```

Each block kind defines whether it is retryable, whether it holds an active
stage, and whether a new workflow event can resume the workflow run.

### 7.4 Issue And PR Refs

```rust
pub struct GithubIssueRef {
    pub owner: String,
    pub repo: String,
    pub number: u64,
    pub node_id: Option<String>,
    pub url: String,
    pub default_branch: String,
}

pub struct GithubPullRequestRef {
    pub owner: String,
    pub repo: String,
    pub number: u64,
    pub node_id: Option<String>,
    pub url: String,
    pub head_branch: String,
    pub head_sha: Option<String>,
}
```

The workspace session is the source of truth for local path, base SHA, current
head SHA, and mount details.

## 8. Workflow Event Ingress Model

### 8.1 Workflow Event Ingress Envelope

All ingress sources must normalize observations into the same envelope:

```rust
pub struct WorkflowEventEnvelope<TPayload> {
    pub source_kind: WorkflowEventSourceKind,
    pub source_delivery_id: Option<String>,
    pub provider: GithubProviderRef,
    pub observed_at: Timestamp,
    pub provider_updated_at: Option<Timestamp>,
    pub idempotency_key: String,
    pub payload_schema: String,
    pub payload: TPayload,
}
```

Source kinds:

```text
poller
github_webhook
benchmark_webhook
manual_operator
workflow_internal
```

Cron and future webhooks both use this envelope. The workflow policy does not depend on
scan order, pagination order, or webhook delivery order.

### 8.2 Workflow events

Workflow events are append-only durable stimuli. They are ordered per workflow run.

```rust
pub struct GithubIssueWorkflowEvent {
    pub workflow_event_id: GithubIssueWorkflowEventId,
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub sequence: i64,
    pub workflow_event_type: GithubIssueWorkflowEventType,
    pub idempotency_key: String,
    pub source_kind: WorkflowEventSourceKind,
    pub source_delivery_id: Option<String>,
    pub provider_node_id: Option<String>,
    pub provider_updated_at: Option<Timestamp>,
    pub observed_at: Timestamp,
    pub supersedes_workflow_event_id: Option<GithubIssueWorkflowEventId>,
    pub payload_schema: String,
    pub payload: JsonValue,
    pub created_at: Timestamp,
}
```

Each MVP workflow event type must have a typed payload contract and a deterministic
idempotency rule.

Examples:

| Workflow Event | Idempotency key |
| --- | --- |
| `github.issue.discovered` | `issue:<node_or_owner_repo_number>:discovered` |
| `github.issue.changed` | `issue:<node_or_owner_repo_number>:updated:<provider_updated_at>` |
| `github.pr.opened` | `pr:<node_or_owner_repo_number>:opened` |
| `github.checks.failed` | `checks:<head_sha>:<suite_or_run_id>:<conclusion>` |
| `github.review_comment.created` | `review-comment:<comment_node_id>` |
| `stage.result.reported` | `stage-result:<stage_run_id>:<schema_version>` |

Older provider events may still be recorded, but workflow policy transition guards must
ignore workflow events superseded by newer provider timestamps or terminal workflow run
state.

### 8.3 Initial Workflow Event Types

```text
github.issue.discovered
github.issue.changed
github.issue.closed

github.pr.opened
github.pr.updated
github.pr.merged
github.pr.closed

github.checks.failed
github.checks.succeeded

github.review_comment.created

stage.triage.completed
stage.plan.completed
stage.implementation.completed
stage.pr_synthesis.completed
stage.ci_repair.completed
stage.review_response.completed
stage.failed

provider_action.succeeded
provider_action.needs_reconciliation
provider_action.failed

workflow_run.blocked
workflow_run.cancelled
```

## 9. Provider Bindings And Echo Suppression

Provider action records are not enough to route later provider events. The
workflow needs a provider-binding table.

```rust
pub struct GithubIssueProviderBinding {
    pub binding_id: GithubIssueProviderBindingId,
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub system: String,          // github
    pub resource_type: String,   // issue | pull_request | comment | check_run
    pub role: String,            // primary | claim | primary_pr | review_thread
    pub owner: String,
    pub repo: String,
    pub provider_id: String,
    pub provider_url: Option<String>,
    pub created_by_provider_action_id: Option<GithubIssueProviderActionId>,
    pub created_at: Timestamp,
}
```

Uses:

- route future webhook deliveries to a workflow run;
- route cron observations to an existing workflow run;
- suppress self-authored echoes;
- reconcile ambiguous provider writes;
- prove which PR/comment/check belongs to which workflow run.

Echo suppression rules:

- classify provider actor on every inbound event;
- ignore self-authored writes when the event exactly matches a known provider action
  or provider binding;
- still record workflow events for provider state transitions caused by others;
- dedupe by source delivery id and workflow event idempotency key.

Without this, later webhooks will be difficult and bot replies may trigger
their own repair/review loops.

## 10. Repository Atomicity And Leases

The repository must expose atomic methods rather than loose CRUD.

Required methods:

```rust
async fn create_or_get_workflow_run(input) -> Result<GithubIssueWorkflowRun>;

async fn record_workflow_event(envelope) -> Result<RecordWorkflowEventOutcome>;

async fn claim_runnable_workflow_runs(
    worker_id: WorkflowWorkerId,
    lease_until: Timestamp,
    limit: usize,
) -> Result<Vec<GithubIssueWorkflowRun>>;

async fn renew_workflow_run_lease(
    workflow_run_id: GithubIssueWorkflowRunId,
    worker_id: WorkflowWorkerId,
    lease_until: Timestamp,
) -> Result<LeaseRenewalOutcome>;

async fn advance_event_cursor_and_transition(
    workflow_run_id: GithubIssueWorkflowRunId,
    expected_version: i64,
    expected_cursor: i64,
    transition: WorkflowRunTransition,
) -> Result<TransitionOutcome>;

async fn release_or_block_workflow_run(input) -> Result<()>;
```

Durable backend requirements:

- unique workflow run key per tenant;
- unique workflow event idempotency key per workflow run;
- monotonically increasing workflow event sequence per workflow run;
- compare-and-swap workflow event cursor advancement;
- workflow run version compare-and-swap;
- lease owner and lease expiry on runnable workflow runs;
- unique active stage per workflow run;
- tenant-scoped list/query methods;
- provider action record uniqueness by `(workflow_run_id, idempotency_key)`;
- provider binding uniqueness by provider ref and role.

This is the IronClaw-native scheduler/step invariant set. It
prevents dual pollers, rolling deploys, and crash retries from starting two
stage runs for the same workflow event.

## 11. Durable Workflow Steps

Workflow policy handlers should not perform arbitrary side effects inline. Each
side-effecting block is a workflow step:

```rust
pub struct WorkflowStepRun {
    pub step_run_id: WorkflowStepRunId,
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub step_name: String,
    pub idempotency_key: String,
    pub input_hash: String,
    pub status: WorkflowStepStatus,
    pub result: Option<JsonValue>,
    pub error: Option<JsonValue>,
    pub started_at: Timestamp,
    pub completed_at: Option<Timestamp>,
}
```

Rules:

- step names are unique within one workflow policy handler;
- step body writes are individually idempotent;
- completed step results may be replayed;
- step failure classification controls retry/block behavior;
- step runs are audit/provenance, not the sole control plane;
- any runnable follow-up work is committed with the state/workflow event transition that
  created it, or can be deterministically recovered.

MVP steps:

```text
claim_issue
start_stage
prepare_workspace
create_or_update_pr
comment_issue
reconcile_provider_action
block_workflow_run
complete_workflow_run
```

## 12. Provider Action Records

Provider actions wrap external writes and ambiguous external reads.

```rust
pub struct GithubIssueProviderActionRecord {
    pub provider_action_id: GithubIssueProviderActionId,
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub stage_run_id: Option<GithubIssueStageRunId>,
    pub step_run_id: Option<WorkflowStepRunId>,
    pub name: String,
    pub idempotency_key: String,
    pub input_hash: String,
    pub status: ProviderActionStatus,
    pub provider_ref_kind: Option<String>,
    pub provider_ref: Option<String>,
    pub stable_marker: Option<String>,
    pub reconciliation_strategy: String,
    pub lease_owner: Option<WorkflowWorkerId>,
    pub lease_expires_at: Option<Timestamp>,
    pub attempt_count: u32,
    pub next_attempt_at: Option<Timestamp>,
    pub last_reconciled_at: Option<Timestamp>,
    pub result: Option<JsonValue>,
    pub redacted_failure_kind: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}
```

Statuses:

```text
pending
running
succeeded
failed
reconciling
needs_reconciliation
```

MVP provider action strategies:

| Provider action | Reconciliation strategy |
| --- | --- |
| claim comment | find issue comment by stable marker |
| issue comment | find issue comment by stable marker |
| branch create/push | inspect expected branch/ref and head SHA |
| draft PR create | find PR by head branch plus body marker |
| review reply | find reply by parent comment/thread marker |

MVP does not require label add/remove, PR update, review-thread resolve, or deep
check-log retrieval unless new GitHub provider capabilities are added.

## 13. Stage Runs And Stage Tasks

Stage runs are the workflow application's execution envelope for model work.
Each stage run is submitted to IronClaw as normal scoped agent work. They are
not a perfect equivalent to fine-grained task chains.

For MVP, each major lifecycle stage is one stage run:

```text
triage
planning
implementation
pr_synthesis
ci_repair
review_response
```

IronClaw already has subagent delegation through the normal turn system:
`builtin.spawn_subagent` creates a parent-owned child thread/turn, blocks the
parent turn on a dependent-run gate, then returns the child terminal result as
capability output to the parent. The GitHub issue workflow should treat that as
execution behavior inside a stage turn, not as the workflow policy itself.

This means the workflow application does not need to port or reimplement
subagent machinery. It needs to define how fan-out is allowed for each stage:
which subagent kinds may be used, how many child runs are allowed, whether
nesting is allowed, and whether child output may influence the sealed parent
stage result.

The default MVP policy should make ad hoc subagents read-only. This mirrors the
latest Stevie orchestrator lesson: subagents are excellent context firewalls
for exploration, review, and analysis, but writable child agents sharing one
checkout create parallel-write and replay hazards. If the workflow needs
parallel writers, it should model them as explicit workflow-managed child stage
tasks with isolated workspace/session ownership, not as model-chosen ad hoc
subagent calls.

That distinction matters:

- a stage turn may use subagents for exploration, planning, review, or
  test/log investigation;
- child subagent turns remain owned by the parent stage turn and IronClaw's
  existing dependent-run gate machinery;
- subagent terminal output does not become a workflow event directly;
- the workflow policy advances only from workflow-owned events, especially the
  sealed `workflow.report_stage_result` result and turn/stage terminal refresh;
- subagent limits, nesting, and capability allowlists stay in IronClaw run
  profile/capability-surface policy;
- the workflow application chooses the stage-level fan-out budget and validates
  that the parent stage result satisfies the schema regardless of how many
  child runs were spawned;
- default child subagents receive read/search/test-inspection capabilities only;
- writer fan-out requires workflow-created child stage tasks with explicit
  workspace isolation, merge/reconcile policy, and stage-result validation.

The design preserves room for fine-grained child stage tasks:

```rust
pub struct GithubIssueStageTask {
    pub stage_task_id: GithubIssueStageTaskId,
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub key: String,
    pub title: String,
    pub status: StageTaskStatus,
    pub stage_run_id: Option<GithubIssueStageRunId>,
    pub input: JsonValue,
    pub result: Option<JsonValue>,
}
```

MVP planning may create stage tasks for traceability, but it does not need to
dispatch every plan item as a separate turn. If implementation quality requires
finer-grained retries later, stage tasks can become workflow-owned child stage
runs or workflow-managed child turns. That should be an explicit workflow
feature, not accidental reliance on a model choosing to spawn subagents.

### Stage Run Record

```rust
pub struct GithubIssueStageRun {
    pub stage_run_id: GithubIssueStageRunId,
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub stage: GithubIssueStage,
    pub status: StageRunStatus,
    pub stage_turn_identity: StageTurnIdentity,
    pub turn_run_id: Option<TurnRunId>,
    pub thread_id: Option<ThreadId>,
    pub prompt_ref: String,
    pub prompt_version: String,
    pub capability_profile_id: String,
    pub capability_profile_version: String,
    pub input_snapshot_hash: String,
    pub result: Option<JsonValue>,
    pub error: Option<StageRunError>,
    pub started_at: Timestamp,
    pub completed_at: Option<Timestamp>,
}
```

Status:

```text
queued
submitting
running
succeeded
failed
blocked
cancelled
```

## 14. Stage Turn Submission

Stage turns are normal scoped IronClaw turns. They are submitted through a
workflow-facing facade that stages canonical content and calls the existing
turn machinery.

```rust
pub struct StageTurnIdentity {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub stage_run_id: GithubIssueStageRunId,
    pub stage: GithubIssueStage,
    pub attempt: u32,
    pub workflow_policy_version: String,
}
```

The identity deterministically derives:

- route/thread key for this stage run;
- external event/idempotency key;
- product context correlation ids.

The submitter must replay safely:

- if the same stage identity was already accepted, return the original
  `ThreadId`/`TurnRunId`;
- if a crash happens after turn acceptance but before storing `turn_run_id`,
  reconcile by looking up the turn by stage identity/product context;
- never submit a second turn for the same active stage unless the workflow policy
  explicitly creates a new attempt.

Workflow port shape:

```rust
pub struct SubmitStageTurnRequest {
    pub stage_turn_identity: StageTurnIdentity,
    pub scope: WorkflowActorScope,
    pub content_ref: WorkflowPromptContentRef,
    pub capability_profile_id: String,
    pub workspace_mount_ref: Option<WorkflowWorkspaceMountRef>,
    pub idempotency_key: String,
}
```

The workflow crate should not pass raw prompt strings, raw tool inputs, or host
paths into `ironclaw_turns`.

If a stage turn spawns subagents, the stage turn remains the workflow-visible
unit. The workflow refresh loop may observe that the stage turn is blocked on a
dependent-run gate, but it must not advance the GitHub issue workflow until the
stage turn either reports a valid sealed result or reaches a terminal state the
workflow can classify.

## 15. Sealed Structured Completion

`workflow.report_stage_result` is the workflow application's structured-result
contract with IronClaw. It is a sealed result sink, not a normal free-form tool
available everywhere.

The capability is visible only inside workflow-issued stage turns and is bound
to:

- workflow run id;
- stage run id;
- turn run id;
- stage;
- result schema version;
- completion nonce or equivalent unforgeable invocation binding.

Payload:

```rust
pub struct ReportStageResultInput {
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub stage_run_id: GithubIssueStageRunId,
    pub turn_run_id: TurnRunId,
    pub stage: GithubIssueStage,
    pub schema_version: String,
    pub completion_nonce: String,
    pub result: JsonValue,
}
```

Rules:

- verify the stage is currently active;
- reject mismatched workflow run/stage/run ids;
- first valid result wins;
- duplicate identical reports replay the accepted result;
- stale reports from older attempts are rejected;
- invalid payloads record validation failure;
- a valid result may be staged before turn terminality, but the workflow policy consumes
  it only when the stage run reaches a valid terminal or accepted-complete
  state;
- model final text is never the source of truth for workflow advancement;
- validation failures are durable workflow facts that can trigger retry,
  repair, or human handoff policy.

Stage schemas should also include framework outcomes:

```text
completed
needs_human
gave_up
exhausted_turns
not_produced
```

Each stage must define a narrower result validator. For example,
implementation cannot report `patch_ready` unless it includes changed files and
test evidence. Verification cannot report success unless it names the commands
or checks that passed. PR synthesis cannot request a PR unless it includes a
branch/head ref and human-readable summary material.

## 16. GitHub Provider Port

The MVP should use one GitHub authority path.

Recommended live path:

- workflow crate calls a narrow `GithubIssueWorkflowPort`;
- composition implements that port using host-runtime mediated GitHub
  capability/egress behavior;
- provider writes are invoked only through workflow provider action records;
- GitHub credential/account selection is host-configured and scoped to the
  workflow actor/repo, never read from issue text or model output.

MVP provider actions:

```rust
async fn search_candidate_bug_issues(input) -> Result<Vec<GithubIssueSummary>>;
async fn search_issues_and_pull_requests(input) -> Result<Vec<GithubSearchResult>>;
async fn get_issue_snapshot(input) -> Result<GithubIssueSnapshot>;
async fn list_issue_comments(input) -> Result<Vec<GithubCommentSnapshot>>;
async fn create_issue_comment(input) -> Result<GithubCommentRef>;
async fn create_branch_or_push(input) -> Result<GithubBranchRef>;
async fn create_draft_pr(input) -> Result<GithubPullRequestRef>;
async fn get_pull_request(input) -> Result<GithubPullRequestSnapshot>;
async fn get_combined_status(input) -> Result<GithubCombinedStatusSnapshot>;
async fn list_review_comments(input) -> Result<Vec<GithubReviewCommentSnapshot>>;
async fn get_authenticated_workflow_actor(input) -> Result<GithubActorSnapshot>;
```

The GitHub extension now also exposes webhook payload handling. Future webhook
support should use that parsing/authentication capability behind a host-owned
ingress normalizer; it should not expose model-visible webhook handling as the
workflow control plane.

Deferred unless capabilities are added:

- label add/remove;
- PR update beyond create/reconcile;
- review thread resolve;
- detailed check-log retrieval;
- merge.

## 17. GitHub Claim Protocol

MVP uses comment-only claiming.

Claim provider action:

1. Create or reuse local workflow run by workflow run key.
2. Reconcile existing claim comments by marker.
3. If no active claim exists, post a claim comment:

   ```markdown
   <!-- ironclaw:bugfix:v1 workflow_run_id=<id> issue=<owner>/<repo>#<n> -->
   IronClaw is attempting this bug fix. A draft PR will be linked here when ready.
   ```

4. Store the comment as:

   - a provider action result;
   - a provider binding with role `claim`;
   - a `github.issue.claimed` workflow event.

If a later poll sees an active claim marker for a different nonterminal
workflow run, skip the issue.

If the marker is stale:

- do not silently reclaim;
- mark local workflow run `blocked_human` if ownership is ambiguous;
- only reclaim with an explicit stale-claim policy.

## 18. Workspace Model

Each active workflow run gets a managed workspace session.

```rust
pub struct GithubIssueWorkspaceSession {
    pub workspace_session_id: GithubIssueWorkspaceSessionId,
    pub workflow_run_id: GithubIssueWorkflowRunId,
    pub repo_ref: GithubRepositoryRef,
    pub base_branch: String,
    pub base_sha: Option<String>,
    pub working_branch: String,
    pub current_head_sha: Option<String>,
    pub remote_url_ref: Option<String>,
    pub mount_ref: WorkflowWorkspaceMountRef,
    pub status: WorkspaceSessionStatus,
    pub lease_owner: Option<WorkflowWorkerId>,
    pub lease_expires_at: Option<Timestamp>,
    pub dirty_state: WorkspaceDirtyState,
    pub archived_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}
```

Domain records store `mount_ref`, not host paths.

Runtime requirements:

- clone/fetch through approved process/network paths;
- mount the workspace as the stage's `/workspace` or equivalent scoped alias;
- enforce one writer at a time per workspace;
- run tests through approved shell/runtime policy;
- prevent symlink, submodule, and path escape through `ScopedFilesystem`;
- archive or mark leaked workspaces durably on terminal workflow run states;
- preserve enough refs to reconcile branch push/PR state after crashes.

## 19. Capability Profiles

Stage capability profiles should map to existing run-profile/capability-surface
machinery, not an invented side channel.

MVP stage surfaces:

```text
triage
  GitHub read: issue + comments
  filesystem: read/search only
  optional read-only spawn_subagent: explorer | reviewer
  workflow.report_stage_result

planning
  GitHub read: issue + comments
  filesystem: read/search only
  optional read-only spawn_subagent: explorer | planner | reviewer
  workflow.report_stage_result

implementation
  filesystem: read/search/write/apply_patch
  shell/test command through runtime policy
  GitHub read only
  optional read-only spawn_subagent: explorer | reviewer
  writer fan-out only through workflow-managed child stage tasks
  workflow.report_stage_result

pr_synthesis
  filesystem: read/search
  no direct GitHub writes
  optional read-only spawn_subagent: planner | reviewer
  workflow.report_stage_result

ci_repair
  GitHub read: PR status/workflow summary
  filesystem: read/search/write/apply_patch
  shell/test command through runtime policy
  optional read-only spawn_subagent: explorer | reviewer
  writer fan-out only through workflow-managed child stage tasks
  workflow.report_stage_result

review_response
  GitHub read: review comments
  filesystem: read/search/write/apply_patch when patching is needed
  shell/test command through runtime policy
  optional read-only spawn_subagent: explorer | reviewer
  writer fan-out only through workflow-managed child stage tasks
  workflow.report_stage_result
```

GitHub writes happen after stage completion via workflow provider actions.
Spawned subagents should not receive `workflow.report_stage_result` by default;
their result returns to the parent stage turn as ordinary dependent-run output.

Tests must prove:

- triage/planning cannot write files;
- implementation cannot create PRs or post comments directly;
- PR synthesis cannot bypass provider action records;
- completion capability rejects wrong workflow run/stage/run ids;
- spawned subagents cannot exceed the parent stage's capability envelope;
- spawned subagents cannot write files or call GitHub write capabilities by
  default;
- workflow-managed child stage tasks, if enabled later, have separate
  workspace/session ownership and merge/reconcile policy;
- subagent completion alone does not advance the workflow policy.

## 20. Workflow Policy Model

The first workflow policy is:

```text
github_issue_bugfix@1
```

The workflow policy is deterministic outside workflow steps. It consumes ordered workflow events
and decides which step to run next.

The policy should support two levels of control:

1. **Deterministic rails** for lifecycle safety: claiming, one active stage,
   provider actions, terminal states, leases, idempotency, and recovery.
2. **Driver-style decisions** for topology and routing when the next move is
   judgment-heavy: plan deltas, whether to split work, when to review, when to
   launch a repair loop, when to park for a human, and when to finish.

The pulled Stevie orchestrator branch reinforces this split. The general lesson
is not to port Stevie's code. It is to avoid turning every workflow into a
rigid hand-coded ladder. The GitHub issue workflow should be able to run a short
driver stage that receives an engineered workflow snapshot and returns a strict
decision:

```json
{
  "plan_delta": [],
  "journal_entries": [],
  "control": {
    "kind": "continue"
  }
}
```

The workflow application validates that decision inside the durable stage/step
boundary, folds invalid outputs into a structured blocked state, and then
applies only the allowed plan/task/provider-action mutations. The model chooses
the proposed topology; the workflow app enforces the rails.

State transitions:

```text
github.issue.discovered
  -> step: claim_issue
  -> mode: claimed
  -> step: start_stage(triage)

stage.triage.completed(can_attempt)
  -> step: start_stage(planning)

stage.triage.completed(needs_human | gave_up | not_produced)
  -> step: block_workflow_run

stage.plan.completed
  -> step: prepare_workspace
  -> step: start_stage(implementation)

stage.implementation.completed(patch_ready)
  -> step: start_stage(pr_synthesis)

stage.implementation.completed(needs_human | exhausted_turns)
  -> step: block_workflow_run

stage.pr_synthesis.completed
  -> step: create_draft_pr
  -> step: comment_issue_with_pr
  -> mode: pr_open

github.checks.failed
  -> guard: workflow run mode is pr_open and no active stage
  -> step: start_stage(ci_repair)

stage.ci_repair.completed(repaired)
  -> step: push_branch_update
  -> mode: pr_open

github.review_comment.created
  -> guard: not self-authored and not already handled
  -> step: start_stage(review_response)

stage.review_response.completed(patched | replied)
  -> step: push_branch_update and/or reply_to_review
  -> mode: pr_open

github.pr.merged
  -> step: complete_workflow_run(succeeded)

github.issue.closed without merged PR
  -> step: complete_workflow_run(cancelled)
```

The workflow policy never calls raw GitHub clients. It calls workflow steps, which call
provider action runners.

## 21. Stage Prompts

Prompts are versioned workflow assets:

```text
prompts/github_issue_bugfix/v1/
  triage.md
  plan.md
  implement.md
  synthesize_pr.md
  repair_ci.md
  address_review.md
```

Each prompt defines:

- stage role;
- source/trust labels for context;
- engineered snapshot shape;
- allowed capability profile;
- exact result schema;
- result narrower;
- stop conditions;
- human-handoff conditions;
- safety instructions for untrusted provider content.

### Context Engineering

Stage prompts and driver prompts must receive engineered snapshots, not raw
database dumps.

Recommended snapshot layout:

- **stable header:** workflow policy version, project/repo identity, issue ref,
  capability profile, and stage goal;
- **durable digest:** issue summary, latest known PR/check/review state,
  current plan/task one-liners, recent journal entries, and provider-action
  statuses;
- **volatile tail:** current workflow event, current stage attempt, known
  blockers, and exact success criteria;
- **lookup references:** ids the agent can inspect through read-only workflow
  or provider lookup capabilities when more detail is needed.

Rules:

- do not dump every comment, fact, tool result, or file into the prompt by
  default;
- provider text is tagged as untrusted data;
- the model-visible snapshot is deterministic for the same workflow event and
  stage input;
- lookup tools are read-only and scoped to the current workflow run/project;
- result schemas stay strict and all framework outcomes are folded
  exhaustively.

### Triage Result

```json
{
  "kind": "can_attempt",
  "summary": "...",
  "suspected_area": "...",
  "reproduction_notes": "...",
  "suggested_tests": ["..."],
  "risk": "low"
}
```

or:

```json
{
  "kind": "needs_human",
  "reason": "...",
  "questions": ["..."]
}
```

### Planning Result

```json
{
  "kind": "planned",
  "plan": [
    {
      "key": "inspect-area",
      "title": "...",
      "rationale": "..."
    }
  ],
  "test_strategy": ["..."],
  "acceptance_criteria": ["..."]
}
```

### Implementation Result

```json
{
  "kind": "patch_ready",
  "summary": "...",
  "files_changed": ["..."],
  "tests_run": [
    { "command": "...", "outcome": "passed", "notes": "..." }
  ],
  "known_risks": ["..."]
}
```

### PR Synthesis Result

```json
{
  "kind": "pr_ready",
  "title": "...",
  "body": "...",
  "issue_closing_keyword": "Fixes #123",
  "ready_for_review": false
}
```

### CI Repair Result

```json
{
  "kind": "repaired",
  "summary": "...",
  "tests_run": ["..."],
  "next_step": "push_update"
}
```

### Review Response Result

```json
{
  "kind": "patched",
  "summary": "...",
  "reply": "..."
}
```

All schemas also support framework outcomes:

```text
needs_human
gave_up
exhausted_turns
not_produced
```

## 22. Prompt Safety

GitHub issue content, comments, PR reviews, CI output, and benchmark traces are
untrusted model input.

Requirements:

- wrap provider content in prompt-envelope style source/trust labels;
- preserve provenance: issue author, comment id, review id, check id;
- hard-cap included comments/log bytes;
- prefer selected snippets and summaries over raw logs;
- state that provider content is evidence, not authority;
- never let provider content alter target repo, branch, capabilities, result
  schema, credential selection, or workflow state.

Trust labels:

```text
workflow/system instructions: authority
repository code/tests: evidence
GitHub issue/comment/review content: untrusted provider content
CI logs: untrusted diagnostic content
benchmark traces: untrusted diagnostic content
```

## 23. Poller Responsibilities

The internal poller does three jobs.

### Discovery

For each configured repository:

1. observe open issues matching the configured selector, default `label:bug`;
2. filter out pull requests;
3. filter out issues with active IronClaw claim markers;
4. create or reuse workflow run records;
5. record `github.issue.discovered`;
6. enqueue/tick runnable workflow runs via repository leases.

### Active Workflow Run Refresh

For nonterminal workflow runs:

1. fetch issue state;
2. fetch primary PR state if linked;
3. fetch combined status/workflow summary for PR head SHA;
4. fetch review comments;
5. normalize observations into workflow event envelopes;
6. record deduped workflow events.

### Stage Run Refresh

For active stage runs:

1. read turn/run state;
2. if running, heartbeat/leave alone;
3. if a valid sealed stage result exists, record completion workflow event;
4. if terminal without valid result, record `stage.failed`;
5. if blocked on approval/auth, set structured block state.

### Backpressure

Poller config:

```text
poll_interval
startup_jitter
tick_jitter
max_repos_per_tick
max_candidates_per_repo
max_active_workflow_runs_per_repo
max_provider_actions_per_tick
max_stage_refreshes_per_tick
```

Persisted scheduling fields:

```text
repo_next_scan_at
rate_limited_until
next_provider_action_attempt_at
next_workflow_run_tick_at
```

GitHub `Retry-After` and secondary rate limits must advance
`rate_limited_until` and avoid retry storms.

## 24. Webhook Readiness

Webhooks become easy if they stop at the workflow event boundary.

Future webhook flow:

```text
GitHub webhook delivery
  -> verify signature
  -> classify actor/self
  -> find workflow run via provider_bindings
  -> build WorkflowEventEnvelope
  -> record_workflow_event()
  -> tick workflow run if runnable
```

No workflow policy changes should be required for:

- issue updates;
- PR updates;
- check suite/run events;
- review comments;
- PR merged/closed events.

Required preparation in MVP:

- provider-binding table exists before webhooks;
- workflow events include source delivery ids;
- echo suppression exists before bot comments/replies;
- workflow policy transition guards handle out-of-order workflow events;
- idempotency keys do not assume polling.

## 25. Error Handling And Recovery

### Retryable

- GitHub network timeout;
- GitHub secondary rate limit;
- transient turn submission failure;
- workspace fetch failure;
- CI status temporarily unavailable;
- provider action lease expired while backend is healthy.

Retry with backoff, preserving workflow run lease/retry state.

### Human-Actionable Or Permanent

- missing GitHub auth;
- repo not configured or not writable;
- issue cannot be handled automatically;
- prompt safety rejection;
- tests cannot run because environment is missing;
- repeated stage schema failures;
- merge conflict requiring decision;
- ambiguous stale claim marker.

Move workflow run to `blocked` with a structured block kind and evidence.

### Ambiguous External Write

Set provider action to `needs_reconciliation`. Reconciler must inspect provider state
by stable marker/ref before retrying.

Examples:

- claim comment timeout -> search comments by marker;
- PR creation timeout -> search PRs by head branch/body marker;
- branch push error -> inspect branch SHA;
- review reply timeout -> search replies by marker.

## 26. Observability And Redaction

Minimum timeline events:

```text
workflow_run.created
workflow event.recorded
workflow_run.lease.claimed
workflow policy.tick.started
workflow policy.tick.completed
step.started
step.completed
provider_action.started
provider_action.succeeded
provider_action.needs_reconciliation
stage.started
stage.turn.submitted
stage.result.reported
stage.completed
stage.failed
workspace.created
workspace.archived
workflow_run.blocked
workflow_run.succeeded
workflow_run.failed
```

Required correlation fields:

```text
tenant_id
creator_user_id
agent_id
project_id
workflow_run_id
workflow_run_version
event_sequence
idempotency_key
worker_id
lease_token
step_run_id
provider_action_id
stage_run_id
turn_run_id
thread_id
workspace_session_id
base_sha
head_sha
provider_ref
attempt_count
next_retry_at
prompt_ref
prompt_version
capability_profile_id
capability_profile_version
redacted_error_kind
last_heartbeat_at
```

Redaction rules:

- no raw secrets;
- no raw host paths;
- no raw provider tokens;
- no unbounded issue comments, review comments, CI logs, prompts, tool args, or
  backend errors in product-visible projections;
- all projections scoped by tenant/user/agent/project/workflow run;
- future WebUI reads go through Reborn/product authorization, not direct store
  reads.

## 27. MVP Slices

### Slice 1 - Domain Skeleton And Fake Workflow Policy

- Add workflow domain crate.
- Add workflow run, workflow event, stage run, provider action, provider binding, and workspace ref
  types.
- Add project-scoped workflow configuration and repository selector types.
- Add in-memory repository.
- Add workflow policy transitions with fake ports.

Acceptance:

- fake discovered workflow event creates a claim step and triage stage intent;
- duplicate workflow events do not double-start a stage;
- terminal/block states stop ticks.

### Slice 2 - Real Stage-Turn Vertical Slice

- Add normal scoped stage-turn submitter facade.
- Add sealed `workflow.report_stage_result`.
- Add one minimal triage prompt/result schema.
- Submit through existing turn infrastructure with a minimal capability
  profile.
- Add an engineered stage snapshot builder and avoid raw state/provider dumps.

Acceptance:

- one workflow-managed stage turn runs;
- valid sealed result advances workflow policy;
- stale/wrong stage result is rejected;
- invalid output produces retry/block state.

### Slice 3 - Cron Discovery And Comment-Only Claim

- Add internal workflow poller.
- Resolve configured repositories from project-scoped workflow configuration.
- Add GitHub candidate search/read/comments/comment provider path.
- Add claim provider action and provider bindings.

Acceptance:

- one tick discovers one unclaimed `bug` issue;
- IronClaw posts one claim comment;
- repeated ticks do not duplicate workflow runs or comments.

### Slice 4 - Managed Workspace And Implementation Stage

- Add workspace materializer.
- Clone/worktree per workflow run.
- Mount workspace into implementation stage.
- Run shell/tests through normal runtime policy.
- Keep ad hoc subagents read-only; defer writer fan-out to explicit
  workflow-managed child stage work with isolated workspace ownership.

Acceptance:

- implementation stage can edit only mounted workspace;
- tests run through approved tools;
- workspace setup failures block with evidence.

### Slice 5 - Draft PR Create/Reconcile

- Add PR synthesis prompt/result schema.
- Push branch through mediated provider action.
- Create draft PR by marker/head branch.
- Comment PR link on issue.

Acceptance:

- successful implementation creates a draft PR;
- retry reuses existing branch/PR by marker;
- issue comment links PR once.

### Slice 6 - Durable Backend Hardening

- Add libSQL/PostgreSQL storage crate.
- Add migrations.
- Add repository contract tests.
- Add lease/CAS/idempotency/reconciliation parity.

Acceptance:

- concurrent ticks cannot start duplicate stages;
- crash-after-submit and crash-after-provider action scenarios reconcile;
- both backends pass parity tests.

### Slice 7 - Lifecycle Refresh V1

- Poll PR state, combined status, workflow runs, and review comments.
- Record lifecycle workflow events.
- Add basic CI repair and review response stages.

Acceptance:

- failed checks start at most one repair stage;
- new review comments start at most one response stage;
- merged PR marks workflow run succeeded.

## 28. Testing Strategy

### Unit Tests

- workflow event idempotency key builders;
- typed payload validation;
- workflow policy transition guards;
- provider action reconciliation strategies;
- stage result schema/narrower validation;
- claim marker parsing;
- prompt input builders.

### Repository Contract Tests

- unique workflow run key;
- workflow event sequence ordering;
- workflow event idempotency;
- workflow run lease claim/renew/release;
- active stage uniqueness;
- step and provider action record lifecycle;
- provider binding uniqueness and lookup;
- tenant-scoped queries;
- libSQL/PostgreSQL parity.

### Caller-Level Tests

Drive the actual workflow call site:

```text
fake GitHub issue
  -> poller tick
  -> workflow event recorded
  -> workflow run claimed
  -> workflow policy tick
  -> stage turn submitted
  -> sealed stage result reported
  -> workflow policy advances
```

### Failure Tests

- crash after claim comment, before provider action success record;
- crash after turn submission, before `turn_run_id` persisted;
- duplicate stage result;
- stale stage result from older attempt;
- two workers claiming same workflow run;
- rate-limit backoff;
- ambiguous PR creation timeout;
- workspace dirty/leaked cleanup.

### E2E / Smoke

- fake GitHub provider for deterministic CI;
- fake turn runner for deterministic stage results;
- live smoke only against a sandbox repo;
- trace assertion connecting issue -> workflow run -> workflow event -> stage -> turn ->
  provider action -> PR.

## 29. Architecture Translation

The design adapts orchestration concepts into a workflow application that uses
IronClaw platform primitives. It does not require all orchestration code to
live in IronClaw core.

| Orchestration concern | IronClaw-native equivalent | Notes |
| --- | --- | --- |
| Durable issue lifecycle | `GithubIssueWorkflowRun` | Status separated from workflow policy state |
| Ordered observations | `GithubIssueWorkflowEvent` | Typed payloads and ingress envelope |
| Deterministic orchestration | `github_issue_bugfix@1` | Workflow policy/state machine |
| Replayable side-effect boundary | `WorkflowStepRun` | Replay/audit/idempotency boundary |
| External write ledger | `GithubIssueProviderActionRecord` | Plus reconciliation and provider bindings |
| Provider resource routing | `GithubIssueProviderBinding` | Needed for future webhooks |
| Agent work envelope | Stage runs + optional stage tasks | Coarser than the source architecture; child tasks can be added |
| Prompted model work | Stage prompt + profile + result schema/narrower | Normal IronClaw turns execute it |
| Driver-style routing | Short workflow decision stage over engineered snapshots | Model proposes plan/topology; workflow app validates and applies |
| In-stage delegation | Existing IronClaw subagent child turns | Parent stage turn consumes child output through dependent-run gates; read-only by default |
| Workspace isolation | Managed workspace session + mount ref | Host paths hidden from domain/product |
| Future push delivery | Future workflow event ingress | Cron records same workflow events for MVP |

Key intentional differences:

- IronClaw owns execution through turns, run profiles, capabilities, subagents,
  status, structured-result plumbing, and runtime/workspace policy.
- The workflow application owns GitHub-specific lifecycle state, prompts,
  result schemas, provider actions, provider bindings, and recovery policy.
- Fine-grained task chains become coarser stage runs for MVP.
- Existing IronClaw subagents remain an execution aid inside stage turns, not
  the workflow lifecycle controller. Default MVP subagents are read-only
  context firewalls.
- Driver-style decisions may choose plan deltas and fan-out, but only inside a
  sealed result/decision schema that the workflow app validates and applies.
- Cron is an internal poller, not an Inngest/webhook edge.
- Provider capabilities already exist in IronClaw, but workflow writes are
  wrapped by provider actions for idempotency.

The risky difference is task granularity. If implementation stages become too
large to retry or inspect, the work-item model should evolve into explicit
workflow-managed child stage runs or child turns. Do not delegate lifecycle
state to ad hoc subagent calls made by the model.

## 30. Rejected Alternatives

### One Giant Cron Prompt

```text
Every few minutes: "Find a bug and fix it."
```

Rejected because it lacks durable lifecycle, idempotent claims, structured
results, provider action reconciliation, workspace ownership, and observability.

### Thin External Harness Around IronClaw

A separate script polls GitHub and drives IronClaw from outside without owning
durable workflow state, strict schemas, provider action records, leases, and
reconciliation.

Rejected because it becomes cron plus prompts plus hope. It proves little about
whether IronClaw exposes enough durable platform primitives for serious
workflow applications.

An external or separately deployed workflow application is acceptable if it
keeps the same contracts this design requires: workflow runs, workflow events,
CAS transitions, stage records, sealed structured results, provider actions,
provider bindings, recovery, and redacted observability. The rejected option is
not "outside the repo"; it is "outside the reliability model."

### Direct GitHub Writes From Agents

Let stage agents call PR/comment/reply tools directly.

Rejected because it bypasses provider action records, reconciliation, provider bindings,
and echo suppression. Agents should propose; workflow provider actions should write.

### Full Generic Orchestration Kernel First

Build a generic workflow run/workflow event/workflow policy engine before the GitHub workflow.

Rejected for MVP. The narrow GitHub issue workflow should prove the seams
before extracting a generic engine.

## 31. Open Decisions

These should be decided before implementation planning:

1. First target repository.
2. Exact candidate selector, defaulting to `label:bug`.
3. Internal poll interval and max active issues per repo.
4. GitHub credential/account selection policy.
5. Branch naming convention.
6. Workspace backend for the first live slice: local clone/worktree vs existing
   sandbox preparation.
7. Whether stage-turn product context needs a new origin kind or a structured
   existing-origin metadata convention.
8. Whether durable adapters live in a separate storage crate from day one or
   start behind features in the workflow crate and split before production.
9. First human handoff surface: GitHub comment, logs, WebUI projection, or a
   combination.
10. Whether project-scoped GitHub automation configuration lives in a workflow
    config table/store or as a typed sub-object in `ProjectRecord.metadata`.

## 32. Recommended MVP Decision Set

- one configured repo;
- project-scoped workflow configuration where available;
- selector `label:bug`;
- one active issue per repo;
- internal fixed-interval poller;
- comment-only visible claim;
- always draft PR;
- normal scoped stage turns;
- sealed structured completion from the first vertical slice;
- agents do not get GitHub write capabilities;
- ad hoc subagents are read-only by default;
- per-workflow-run workspace mounted as scoped `/workspace`;
- fake GitHub and fake turn runner tests before live provider work;
- live smoke only against a sandbox repository.

## 33. Success Criteria

The MVP is successful when:

1. A cron-style poller discovers an open GitHub issue labeled `bug`.
2. IronClaw creates one durable workflow run for that issue.
3. IronClaw posts one visible claim comment.
4. Workflow events, leases, and idempotency prevent duplicate work across repeated ticks.
5. The workflow policy starts triage, planning, implementation, and PR synthesis stages.
6. Each stage runs as a normal scoped IronClaw turn.
7. Each stage records a sealed structured result.
8. Implementation edits only an isolated workspace.
9. Workflow provider actions create or reconcile a draft PR.
10. Failures produce structured blocked states with redacted evidence.
11. A trace connects issue -> workflow run -> workflow events -> steps -> stage runs ->
    turn runs -> provider actions -> provider bindings -> PR.

That is the smallest version that deserves to be called an orchestration MVP
rather than a scheduled prompt.
