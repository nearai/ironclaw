//! Workspace-defined lifecycle hooks.
//!
//! Workspace hooks let users define lightweight, repo-local policies without
//! building a WASM extension. Hook definitions are loaded from workspace
//! documents under `hooks/*.hook.json`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::WorkspaceError;
use crate::hooks::hook::{
    Hook, HookContext, HookError, HookEvent, HookFailureMode, HookOutcome, HookPoint,
};
use crate::hooks::registry::HookRegistry;
use crate::workspace::Workspace;

const WORKSPACE_HOOK_DIR: &str = "hooks/";
const WORKSPACE_HOOK_SUFFIX: &str = ".hook.json";

/// Load and register all workspace hook definitions.
///
/// Hook files are discovered via `Workspace::list_all()` and filtered by the
/// `hooks/*.hook.json` convention.
pub async fn register_workspace_hooks(
    registry: &HookRegistry,
    workspace: &Workspace,
) -> Result<usize, WorkspaceError> {
    let mut paths: Vec<String> = workspace
        .list_all()
        .await?
        .into_iter()
        .filter(|p| p.starts_with(WORKSPACE_HOOK_DIR) && p.ends_with(WORKSPACE_HOOK_SUFFIX))
        .collect();

    if paths.is_empty() {
        return Ok(0);
    }

    paths.sort();

    let mut loaded = 0usize;
    for path in &paths {
        let doc = match workspace.read(path).await {
            Ok(doc) => doc,
            Err(err) => {
                tracing::warn!(path = %path, "Failed to read workspace hook: {}", err);
                continue;
            }
        };

        let spec = match serde_json::from_str::<WorkspaceHookSpec>(&doc.content) {
            Ok(spec) => spec,
            Err(err) => {
                tracing::warn!(
                    path = %path,
                    "Invalid workspace hook JSON (skipping): {}",
                    err
                );
                continue;
            }
        };

        let Some(hook) = WorkspaceInlineHook::from_spec(path, spec) else {
            tracing::warn!(path = %path, "Workspace hook has no valid hook points (skipping)");
            continue;
        };

        let priority = hook.priority;
        registry
            .register_with_priority(Arc::new(hook), priority)
            .await;
        loaded += 1;
    }

    Ok(loaded)
}

#[derive(Debug, Clone, Deserialize)]
struct WorkspaceHookSpec {
    #[serde(default)]
    name: Option<String>,
    points: Vec<String>,
    #[serde(default = "default_fail_open")]
    failure_mode: String,
    #[serde(default = "default_hook_timeout")]
    timeout_ms: u64,
    #[serde(default = "default_hook_priority")]
    priority: u32,
    action: WorkspaceHookActionSpec,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WorkspaceHookActionSpec {
    RejectContains {
        needle: String,
        #[serde(default)]
        reason: Option<String>,
    },
    Replace {
        find: String,
        replace: String,
    },
}

#[derive(Debug, Clone)]
enum WorkspaceHookAction {
    RejectContains { needle: String, reason: String },
    Replace { find: String, replace: String },
}

#[derive(Debug)]
struct WorkspaceInlineHook {
    name: String,
    points: Vec<HookPoint>,
    failure_mode: HookFailureMode,
    timeout: Duration,
    priority: u32,
    action: WorkspaceHookAction,
}

impl WorkspaceInlineHook {
    fn from_spec(path: &str, spec: WorkspaceHookSpec) -> Option<Self> {
        let points: Vec<HookPoint> = spec
            .points
            .iter()
            .filter_map(|p| parse_hook_point(p))
            .collect();
        if points.is_empty() {
            return None;
        }

        let failure_mode = match spec.failure_mode.as_str() {
            "fail_closed" | "failClosed" => HookFailureMode::FailClosed,
            _ => HookFailureMode::FailOpen,
        };

        let action = match spec.action {
            WorkspaceHookActionSpec::RejectContains { needle, reason } => {
                WorkspaceHookAction::RejectContains {
                    needle,
                    reason: reason.unwrap_or_else(|| "Rejected by workspace hook".to_string()),
                }
            }
            WorkspaceHookActionSpec::Replace { find, replace } => {
                WorkspaceHookAction::Replace { find, replace }
            }
        };

        Some(Self {
            name: spec
                .name
                .unwrap_or_else(|| format!("workspace:{}", path.replace('/', ":"))),
            points,
            failure_mode,
            timeout: Duration::from_millis(spec.timeout_ms),
            priority: spec.priority,
            action,
        })
    }
}

#[async_trait]
impl Hook for WorkspaceInlineHook {
    fn name(&self) -> &str {
        &self.name
    }

    fn hook_points(&self) -> &[HookPoint] {
        &self.points
    }

    fn failure_mode(&self) -> HookFailureMode {
        self.failure_mode
    }

    fn timeout(&self) -> Duration {
        self.timeout
    }

    async fn execute(
        &self,
        event: &HookEvent,
        _ctx: &HookContext,
    ) -> Result<HookOutcome, HookError> {
        let Some(content) = event_text_content(event) else {
            return Ok(HookOutcome::ok());
        };

        match &self.action {
            WorkspaceHookAction::RejectContains { needle, reason } => {
                if !needle.is_empty() && content.contains(needle) {
                    Ok(HookOutcome::reject(reason.clone()))
                } else {
                    Ok(HookOutcome::ok())
                }
            }
            WorkspaceHookAction::Replace { find, replace } => {
                if find.is_empty() || !content.contains(find) {
                    return Ok(HookOutcome::ok());
                }

                Ok(HookOutcome::modify(content.replace(find, replace)))
            }
        }
    }
}

fn event_text_content(event: &HookEvent) -> Option<&str> {
    match event {
        HookEvent::Inbound { content, .. } | HookEvent::Outbound { content, .. } => Some(content),
        HookEvent::ResponseTransform { response, .. } => Some(response),
        HookEvent::Parse { parsed_intent, .. } => Some(parsed_intent),
        HookEvent::ToolResult { result, .. } => Some(result),
        HookEvent::ToolCall { .. }
        | HookEvent::ApprovalRequest { .. }
        | HookEvent::SessionStart { .. }
        | HookEvent::SessionEnd { .. }
        | HookEvent::AgenticLoopStart { .. }
        | HookEvent::LlmCall { .. } => None,
    }
}

fn default_fail_open() -> String {
    "fail_open".to_string()
}

fn default_hook_timeout() -> u64 {
    5000
}

fn default_hook_priority() -> u32 {
    100
}

fn parse_hook_point(s: &str) -> Option<HookPoint> {
    match s.to_lowercase().replace('_', "").as_str() {
        "beforeinbound" => Some(HookPoint::BeforeInbound),
        "beforetoolcall" => Some(HookPoint::BeforeToolCall),
        "beforeoutbound" => Some(HookPoint::BeforeOutbound),
        "onsessionstart" => Some(HookPoint::OnSessionStart),
        "onsessionend" => Some(HookPoint::OnSessionEnd),
        "transformresponse" => Some(HookPoint::TransformResponse),
        "afterparse" => Some(HookPoint::AfterParse),
        "beforeagenticloop" => Some(HookPoint::BeforeAgenticLoop),
        "beforellmcall" => Some(HookPoint::BeforeLlmCall),
        "aftertoolcall" => Some(HookPoint::AfterToolCall),
        "beforeapproval" => Some(HookPoint::BeforeApproval),
        other => {
            tracing::warn!("Unknown workspace hook point: {:?}", other);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hook_point_aliases() {
        assert_eq!(
            parse_hook_point("beforeInbound"),
            Some(HookPoint::BeforeInbound)
        );
        assert_eq!(
            parse_hook_point("before_inbound"),
            Some(HookPoint::BeforeInbound)
        );
        assert_eq!(parse_hook_point("afterParse"), Some(HookPoint::AfterParse));
        assert!(parse_hook_point("notARealPoint").is_none());
    }

    #[tokio::test]
    async fn test_workspace_hook_reject_contains() {
        let hook = WorkspaceInlineHook::from_spec(
            "hooks/reject.hook.json",
            WorkspaceHookSpec {
                name: Some("workspace:test-reject".to_string()),
                points: vec!["beforeOutbound".to_string()],
                failure_mode: "fail_open".to_string(),
                timeout_ms: 250,
                priority: 7,
                action: WorkspaceHookActionSpec::RejectContains {
                    needle: "SECRET".to_string(),
                    reason: Some("Contains secret marker".to_string()),
                },
            },
        )
        .unwrap();

        let event = HookEvent::Outbound {
            user_id: "u1".to_string(),
            channel: "test".to_string(),
            content: "this contains SECRET value".to_string(),
            thread_id: None,
        };

        let outcome = hook.execute(&event, &HookContext::default()).await.unwrap();
        assert!(matches!(outcome, HookOutcome::Reject { .. }));
    }

    #[tokio::test]
    async fn test_workspace_hook_replace() {
        let hook = WorkspaceInlineHook::from_spec(
            "hooks/replace.hook.json",
            WorkspaceHookSpec {
                name: Some("workspace:test-replace".to_string()),
                points: vec!["beforeInbound".to_string()],
                failure_mode: "fail_open".to_string(),
                timeout_ms: 250,
                priority: 7,
                action: WorkspaceHookActionSpec::Replace {
                    find: "foo".to_string(),
                    replace: "bar".to_string(),
                },
            },
        )
        .unwrap();

        let event = HookEvent::Inbound {
            user_id: "u1".to_string(),
            channel: "test".to_string(),
            content: "foo baz".to_string(),
            thread_id: None,
        };

        let outcome = hook.execute(&event, &HookContext::default()).await.unwrap();
        match outcome {
            HookOutcome::Continue {
                modified: Some(content),
            } => assert_eq!(content, "bar baz"),
            other => panic!("expected modified outcome, got: {:?}", other),
        }
    }
}
