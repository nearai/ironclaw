//! Project administration tools — create, update, set-active, assign-thread.
//!
//! These are thin dispatcher-compliant wrappers over `crate::bridge` so that
//! gateway handlers creating, updating, or switching projects go through
//! `ToolDispatcher::dispatch()` like every other non-agent mutation (per
//! `.claude/rules/tools.md`). The tools themselves contain no engine or DB
//! logic — that lives in the bridge.
//!
//! The set-active pointer is stored as a workspace MemoryDoc at
//! `projects/_active.json`. Keeping it in workspace (rather than Settings or
//! a new DB column) mirrors how other user-scoped pointer state is handled
//! and costs nothing in persistence schema.
//!
//! **Not implemented here**: `project_delete`. The engine `Store` trait
//! lacks `delete_project` today (`crates/ironclaw_engine/src/traits/store.rs`);
//! adding it requires matching impls across every backend and test store.
//! Users can rename/repurpose a project via `project_update` as a stopgap.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::builtin::memory::WorkspaceResolver;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

const ACTIVE_PROJECT_PATH: &str = "projects/_active.json";

/// Parse the optional `workspace_path` / `github_repo` / `default_branch`
/// fields into the shape `ProjectUpsertFields` expects.
///
/// The tri-state is: key absent → `None` (leave unchanged), key present and
/// `null` or empty → `Some(None)` (clear), key present with a value →
/// `Some(Some(value))` (set).
fn parse_optional_opt_path(
    params: &serde_json::Value,
    key: &str,
) -> Result<Option<Option<PathBuf>>, ToolError> {
    let Some(raw) = params.get(key) else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(Some(None));
    }
    let s = raw
        .as_str()
        .ok_or_else(|| ToolError::InvalidParameters(format!("'{key}' must be a string or null")))?;
    if s.is_empty() {
        return Ok(Some(None));
    }
    Ok(Some(Some(PathBuf::from(s))))
}

fn parse_optional_opt_string(
    params: &serde_json::Value,
    key: &str,
) -> Result<Option<Option<String>>, ToolError> {
    let Some(raw) = params.get(key) else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(Some(None));
    }
    let s = raw
        .as_str()
        .ok_or_else(|| ToolError::InvalidParameters(format!("'{key}' must be a string or null")))?;
    if s.is_empty() {
        return Ok(Some(None));
    }
    Ok(Some(Some(s.to_string())))
}

fn parse_optional_opt_github_repo(
    params: &serde_json::Value,
    key: &str,
) -> Result<Option<Option<ironclaw_common::GitHubRepo>>, ToolError> {
    let Some(raw) = params.get(key) else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(Some(None));
    }
    let s = raw
        .as_str()
        .ok_or_else(|| ToolError::InvalidParameters(format!("'{key}' must be a string or null")))?;
    if s.is_empty() {
        return Ok(Some(None));
    }
    let repo = ironclaw_common::GitHubRepo::new(s)
        .map_err(|e| ToolError::InvalidParameters(e.to_string()))?;
    Ok(Some(Some(repo)))
}

fn upsert_fields_from_params(
    params: &serde_json::Value,
    require_name: bool,
) -> Result<crate::bridge::ProjectUpsertFields, ToolError> {
    let name = if require_name {
        Some(require_str(params, "name")?.to_string())
    } else {
        params
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    let description = params
        .get("description")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let workspace_path = parse_optional_opt_path(params, "workspace_path")?;
    let github_repo = parse_optional_opt_github_repo(params, "github_repo")?;
    let default_branch = parse_optional_opt_string(params, "default_branch")?;

    Ok(crate::bridge::ProjectUpsertFields {
        name,
        description,
        workspace_path,
        github_repo,
        default_branch,
    })
}

fn map_bridge_err(e: crate::error::Error) -> ToolError {
    ToolError::ExecutionFailed(e.to_string())
}

// ── project_create ─────────────────────────────────────────────────

pub struct ProjectCreateTool;

#[async_trait]
impl Tool for ProjectCreateTool {
    fn name(&self) -> &str {
        "project_create"
    }

    fn description(&self) -> &str {
        "Create a new coding project that binds a conversation to a host folder and optionally a GitHub repo. \
         The project's workspace_path defaults to ~/.ironclaw/projects/<user_id>/<project_id>/ if omitted; \
         when a custom path is supplied it becomes the project's root. GitHub repo must be in owner/repo form."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "Human-readable project name."},
                "description": {"type": "string", "description": "Optional short description."},
                "workspace_path": {
                    "type": ["string", "null"],
                    "description": "Optional absolute host path to use as the project root. When omitted, defaults to ~/.ironclaw/projects/<user_id>/<project_id>/."
                },
                "github_repo": {
                    "type": ["string", "null"],
                    "description": "Optional GitHub repo in 'owner/repo' form (e.g. 'nearai/ironclaw')."
                },
                "default_branch": {
                    "type": ["string", "null"],
                    "description": "Optional default target branch for PRs. Defaults to 'staging' when used by the coding skill."
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let fields = upsert_fields_from_params(&params, true)?;
        let info = crate::bridge::create_engine_project(&ctx.user_id, fields)
            .await
            .map_err(map_bridge_err)?;
        let body =
            serde_json::to_string(&info).map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        Ok(ToolOutput::text(body, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ── project_update ─────────────────────────────────────────────────

pub struct ProjectUpdateTool;

#[async_trait]
impl Tool for ProjectUpdateTool {
    fn name(&self) -> &str {
        "project_update"
    }

    fn description(&self) -> &str {
        "Update an existing project. All non-id fields are optional; pass null or empty string to clear \
         github_repo / default_branch / workspace_path back to their defaults."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Project UUID."},
                "name": {"type": "string"},
                "description": {"type": "string"},
                "workspace_path": {"type": ["string", "null"]},
                "github_repo": {"type": ["string", "null"]},
                "default_branch": {"type": ["string", "null"]}
            },
            "required": ["id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let id = require_str(&params, "id")?;
        let fields = upsert_fields_from_params(&params, false)?;
        let info = crate::bridge::update_engine_project(id, &ctx.user_id, fields)
            .await
            .map_err(map_bridge_err)?;
        let body =
            serde_json::to_string(&info).map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        Ok(ToolOutput::text(body, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ── project_set_active ─────────────────────────────────────────────

pub struct ProjectSetActiveTool {
    resolver: Arc<dyn WorkspaceResolver>,
}

impl ProjectSetActiveTool {
    pub fn new(resolver: Arc<dyn WorkspaceResolver>) -> Self {
        Self { resolver }
    }
}

#[async_trait]
impl Tool for ProjectSetActiveTool {
    fn name(&self) -> &str {
        "project_set_active"
    }

    fn description(&self) -> &str {
        "Set the active project for this user. New threads inherit the active project unless \
         an explicit per-thread override is set. Stored as a workspace MemoryDoc at projects/_active.json. \
         Passing a null project_id clears the active pointer."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "project_id": {"type": ["string", "null"], "description": "Project UUID. Null clears the active pointer."}
            },
            "required": []
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let workspace = self.resolver.resolve(&ctx.user_id).await;
        let project_id = params.get("project_id").and_then(|v| v.as_str());

        let content = match project_id {
            Some(id) if !id.is_empty() => {
                // Validate the project exists and is owned by this user before
                // storing the pointer. A dangling active pointer would surface
                // as a mysterious "no project" chrome bug later.
                let maybe = crate::bridge::get_engine_project(id, &ctx.user_id)
                    .await
                    .map_err(map_bridge_err)?;
                if maybe.is_none() {
                    return Err(ToolError::InvalidParameters(format!(
                        "project '{id}' not found or not owned by this user"
                    )));
                }
                serde_json::to_string(&serde_json::json!({ "project_id": id }))
                    .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?
            }
            _ => "{}".to_string(),
        };

        workspace
            .write(ACTIVE_PROJECT_PATH, &content)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let result = serde_json::json!({ "project_id": project_id });
        Ok(ToolOutput::text(result.to_string(), start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ── project_assign_thread ──────────────────────────────────────────

pub struct ProjectAssignThreadTool;

#[async_trait]
impl Tool for ProjectAssignThreadTool {
    fn name(&self) -> &str {
        "project_assign_thread"
    }

    fn description(&self) -> &str {
        "Bind a conversation/thread to a specific project, overriding the user's active project for that \
         thread. Pass a null project_id to clear the override so the thread follows the active project again."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "thread_id": {"type": "string", "description": "Conversation UUID."},
                "project_id": {"type": ["string", "null"], "description": "Project UUID to assign. Null clears the override."}
            },
            "required": ["thread_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let thread_id_str = require_str(&params, "thread_id")?;
        let thread_id = uuid::Uuid::parse_str(thread_id_str)
            .map_err(|e| ToolError::InvalidParameters(format!("thread_id: {e}")))?;

        let project_id = params
            .get("project_id")
            .and_then(|v| if v.is_null() { None } else { v.as_str() });

        // If a project_id is provided, validate existence + ownership before
        // writing anything. This prevents the thread-level metadata row from
        // referencing a project the user does not own.
        if let Some(id) = project_id {
            let maybe = crate::bridge::get_engine_project(id, &ctx.user_id)
                .await
                .map_err(map_bridge_err)?;
            if maybe.is_none() {
                return Err(ToolError::InvalidParameters(format!(
                    "project '{id}' not found or not owned by this user"
                )));
            }
        }

        crate::bridge::set_conversation_project(thread_id, project_id)
            .await
            .map_err(map_bridge_err)?;

        let result = serde_json::json!({
            "thread_id": thread_id_str,
            "project_id": project_id,
        });
        Ok(ToolOutput::text(result.to_string(), start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}
