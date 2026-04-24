//! `thread_metadata_set` — patch the current thread's metadata object.
//!
//! The tool itself does not persist anything. It validates the `patch`
//! argument is an object with string keys and echoes it back as the
//! tool's output payload. The engine's `handle_execute_action` in
//! `crates/ironclaw_engine/src/executor/orchestrator.rs` reads the
//! output and applies a replace-at-top-level-key merge into the
//! in-memory `thread.metadata`. Persistence happens on the next thread
//! save, same as any other in-flight mutation.
//!
//! Replace-at-top-level-key (not deep merge) is the semantics by design:
//! skills namespace their state under a single key (e.g. `dev`,
//! `meeting_notes`) and each write overwrites that namespace wholesale.
//! Deep merge lets a skill silently drop keys from another skill's
//! namespace when both write at once; replace-at-top makes contention
//! visible.

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput};

pub struct ThreadMetadataSetTool;

#[async_trait]
impl Tool for ThreadMetadataSetTool {
    fn name(&self) -> &str {
        "thread_metadata_set"
    }

    fn description(&self) -> &str {
        "Patch the current thread's metadata. Pass a JSON object whose top-level keys replace the \
         matching keys in the thread's metadata (replace-at-top-level, not deep-merge). Use a \
         per-skill namespace key (e.g. { \"dev\": { \"branch\": \"...\", \"pr_url\": \"...\" } }) \
         so skills do not overwrite each other. The current metadata is visible in the \
         thread_state system message on each turn."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "patch": {
                    "type": "object",
                    "description": "JSON object whose top-level keys overwrite the matching keys in thread.metadata."
                }
            },
            "required": ["patch"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let patch = params.get("patch").ok_or_else(|| {
            ToolError::InvalidParameters("missing 'patch' object parameter".into())
        })?;
        let obj = patch
            .as_object()
            .ok_or_else(|| ToolError::InvalidParameters("'patch' must be a JSON object".into()))?;

        // The engine side applies the patch; guard against runaway sizes here
        // so a skill cannot bloat context by writing megabytes of metadata.
        let patch_str = serde_json::to_string(&serde_json::Value::Object(obj.clone()))
            .map_err(|e| ToolError::ExecutionFailed(format!("serialize patch: {e}")))?;
        const MAX_PATCH_BYTES: usize = 8 * 1024;
        if patch_str.len() > MAX_PATCH_BYTES {
            return Err(ToolError::InvalidParameters(format!(
                "patch too large ({} bytes, max {})",
                patch_str.len(),
                MAX_PATCH_BYTES
            )));
        }

        // The output payload IS the patch — the orchestrator reads it back
        // out by action name and applies it to the in-memory thread.
        Ok(ToolOutput::text(patch_str, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> JobContext {
        JobContext::new("t", "d")
    }

    #[tokio::test]
    async fn rejects_missing_patch() {
        let t = ThreadMetadataSetTool;
        let err = t.execute(serde_json::json!({}), &ctx()).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidParameters(_)));
    }

    #[tokio::test]
    async fn rejects_non_object_patch() {
        let t = ThreadMetadataSetTool;
        let err = t
            .execute(serde_json::json!({"patch": "nope"}), &ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidParameters(_)));
    }

    #[tokio::test]
    async fn rejects_oversized_patch() {
        let t = ThreadMetadataSetTool;
        let huge = "x".repeat(9000);
        let err = t
            .execute(serde_json::json!({"patch": {"k": huge}}), &ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidParameters(_)));
    }

    #[tokio::test]
    async fn echoes_patch_as_output() {
        let t = ThreadMetadataSetTool;
        let out = t
            .execute(
                serde_json::json!({"patch": {"dev": {"branch": "feature/x"}}}),
                &ctx(),
            )
            .await
            .unwrap();
        let body = out.result.as_str().expect("text output");
        let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(parsed["dev"]["branch"], "feature/x");
    }
}
