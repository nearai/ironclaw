use ironclaw_github_issue_workflow::{
    GithubIssueProviderActionId, GithubIssueProviderBindingId, GithubIssueStageRunId,
    GithubIssueWorkflowError, GithubIssueWorkflowRunId, GithubIssueWorkflowRunKey,
    GithubProviderRef, WorkflowIdempotencyKey, WorkflowStepRunId,
};
use ironclaw_host_api::{ResourceScope, ScopedPath};
use sha2::{Digest, Sha256};

use super::durable_error;

const DEFAULT_REPOSITORY_ROOT: &str = "/engine/github_issue_workflow";

pub(super) fn default_scoped_repository_root() -> ScopedPath {
    ScopedPath::new(DEFAULT_REPOSITORY_ROOT)
        .expect("default GitHub issue workflow root is a valid scoped path") // safety: DEFAULT_REPOSITORY_ROOT is a static absolute path under /engine.
}

pub(super) fn scoped_repository_root_for_scope(
    root: ScopedPath,
    scope: &ResourceScope,
) -> ScopedPath {
    let agent_id = scope
        .agent_id
        .as_ref()
        .map(|agent_id| agent_id.as_str())
        .unwrap_or("_");
    let project_id = scope
        .project_id
        .as_ref()
        .map(|project_id| project_id.as_str())
        .unwrap_or("_");
    let mission_id = scope
        .mission_id
        .as_ref()
        .map(|mission_id| mission_id.as_str())
        .unwrap_or("_");
    let thread_id = scope
        .thread_id
        .as_ref()
        .map(|thread_id| thread_id.as_str())
        .unwrap_or("_");
    let path = format!(
        "{}/_scope/{}/{}/{}/{}/{}/{}",
        root.as_str().trim_end_matches('/'),
        hex_component(scope.tenant_id.as_str()),
        hex_component(scope.user_id.as_str()),
        hex_component(agent_id),
        hex_component(project_id),
        hex_component(mission_id),
        hex_component(thread_id)
    );
    ScopedPath::new(path).expect("scope-partitioned repository root is a valid scoped path") // safety: the root is valid and every appended component is hex-encoded.
}

pub(super) fn runs_root(root: &ScopedPath) -> Result<ScopedPath, GithubIssueWorkflowError> {
    child_path(root, "runs")
}

pub(super) fn run_path(
    root: &ScopedPath,
    tenant_id: &ironclaw_host_api::TenantId,
    workflow_run_id: &GithubIssueWorkflowRunId,
) -> Result<ScopedPath, GithubIssueWorkflowError> {
    child_path(
        root,
        &format!(
            "runs/{}/{}.json",
            hex_component(tenant_id.as_str()),
            hex_component(workflow_run_id.as_str())
        ),
    )
}

pub(super) fn run_key_path(
    root: &ScopedPath,
    tenant_id: &ironclaw_host_api::TenantId,
    workflow_run_key: &GithubIssueWorkflowRunKey,
) -> Result<ScopedPath, GithubIssueWorkflowError> {
    child_path(
        root,
        &format!(
            "run_keys/{}/{}.json",
            hex_component(tenant_id.as_str()),
            hash_component(&[workflow_run_key.as_str()])
        ),
    )
}

pub(super) fn event_sequence_path(
    root: &ScopedPath,
    workflow_run_id: &GithubIssueWorkflowRunId,
) -> Result<ScopedPath, GithubIssueWorkflowError> {
    child_path(
        root,
        &format!(
            "event_sequences/{}.json",
            hex_component(workflow_run_id.as_str())
        ),
    )
}

pub(super) fn events_run_root(
    root: &ScopedPath,
    workflow_run_id: &GithubIssueWorkflowRunId,
) -> Result<ScopedPath, GithubIssueWorkflowError> {
    child_path(
        root,
        &format!("events/{}", hex_component(workflow_run_id.as_str())),
    )
}

pub(super) fn event_path(
    root: &ScopedPath,
    workflow_run_id: &GithubIssueWorkflowRunId,
    sequence: i64,
) -> Result<ScopedPath, GithubIssueWorkflowError> {
    child_path(
        root,
        &format!(
            "events/{}/{:020}.json",
            hex_component(workflow_run_id.as_str()),
            sequence
        ),
    )
}

pub(super) fn event_key_path(
    root: &ScopedPath,
    workflow_run_id: &GithubIssueWorkflowRunId,
    idempotency_key: &WorkflowIdempotencyKey,
) -> Result<ScopedPath, GithubIssueWorkflowError> {
    child_path(
        root,
        &format!(
            "event_keys/{}/{}.json",
            hex_component(workflow_run_id.as_str()),
            hash_component(&[idempotency_key.as_str()])
        ),
    )
}

pub(super) fn stage_path(
    root: &ScopedPath,
    workflow_run_id: &GithubIssueWorkflowRunId,
    stage_run_id: &GithubIssueStageRunId,
) -> Result<ScopedPath, GithubIssueWorkflowError> {
    child_path(
        root,
        &format!(
            "stages/{}/{}.json",
            hex_component(workflow_run_id.as_str()),
            hex_component(stage_run_id.as_str())
        ),
    )
}

pub(super) fn step_path(
    root: &ScopedPath,
    workflow_run_id: &GithubIssueWorkflowRunId,
    idempotency_key: &WorkflowIdempotencyKey,
) -> Result<ScopedPath, GithubIssueWorkflowError> {
    child_path(
        root,
        &format!(
            "steps/{}/{}.json",
            hex_component(workflow_run_id.as_str()),
            hash_component(&[idempotency_key.as_str()])
        ),
    )
}

pub(super) fn provider_action_path(
    root: &ScopedPath,
    workflow_run_id: &GithubIssueWorkflowRunId,
    idempotency_key: &WorkflowIdempotencyKey,
) -> Result<ScopedPath, GithubIssueWorkflowError> {
    child_path(
        root,
        &format!(
            "provider_actions/{}/{}.json",
            hex_component(workflow_run_id.as_str()),
            hash_component(&[idempotency_key.as_str()])
        ),
    )
}

pub(super) fn provider_action_id_path(
    root: &ScopedPath,
    provider_action_id: &GithubIssueProviderActionId,
) -> Result<ScopedPath, GithubIssueWorkflowError> {
    child_path(
        root,
        &format!(
            "provider_action_ids/{}.json",
            hex_component(provider_action_id.as_str())
        ),
    )
}

pub(super) fn provider_binding_path(
    root: &ScopedPath,
    provider_ref: &GithubProviderRef,
    role: &str,
) -> Result<ScopedPath, GithubIssueWorkflowError> {
    child_path(
        root,
        &format!(
            "provider_bindings/{}.json",
            provider_binding_hash(provider_ref, role)
        ),
    )
}

pub(super) fn provider_binding_id_path(
    root: &ScopedPath,
    binding_id: &GithubIssueProviderBindingId,
) -> Result<ScopedPath, GithubIssueWorkflowError> {
    child_path(
        root,
        &format!(
            "provider_binding_ids/{}.json",
            hex_component(binding_id.as_str())
        ),
    )
}

pub(super) fn workflow_step_id_path(
    root: &ScopedPath,
    step_run_id: &WorkflowStepRunId,
) -> Result<ScopedPath, GithubIssueWorkflowError> {
    child_path(
        root,
        &format!(
            "workflow_step_ids/{}.json",
            hex_component(step_run_id.as_str())
        ),
    )
}

fn child_path(root: &ScopedPath, suffix: &str) -> Result<ScopedPath, GithubIssueWorkflowError> {
    let path = format!("{}/{}", root.as_str().trim_end_matches('/'), suffix);
    ScopedPath::new(path).map_err(|error| durable_error("construct storage path", error))
}

fn provider_binding_hash(provider_ref: &GithubProviderRef, role: &str) -> String {
    hash_component(&[
        &provider_ref.system,
        &provider_ref.resource_type,
        role,
        &provider_ref.owner,
        &provider_ref.repo,
        &provider_ref.provider_id,
    ])
}

fn hash_component(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.len().to_be_bytes());
        hasher.update(part.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn hex_component(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}
