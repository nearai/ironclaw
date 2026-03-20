use std::collections::HashSet;
use std::sync::Arc;

use crate::extensions::ExtensionManager;

use super::ToolRegistry;

pub const AUTONOMOUS_TOOL_DENYLIST: &[&str] = &[
    "routine_create",
    "routine_update",
    "routine_delete",
    "routine_fire",
    "event_emit",
    "create_job",
    "job_prompt",
    "restart",
    "tool_install",
    "tool_auth",
    "tool_activate",
    "tool_remove",
    "tool_upgrade",
    "skill_install",
    "skill_remove",
    "secret_list",
    "secret_delete",
];

pub fn is_autonomous_tool_denylisted(tool_name: &str) -> bool {
    AUTONOMOUS_TOOL_DENYLIST.contains(&tool_name)
}

pub fn autonomous_unavailable_message(tool_name: &str, owner_id: &str) -> String {
    if is_autonomous_tool_denylisted(tool_name) {
        format!("Tool '{tool_name}' is not available in autonomous jobs or routines")
    } else {
        format!("Tool '{tool_name}' is not currently available for owner '{owner_id}'")
    }
}

pub fn autonomous_unavailable_error(tool_name: &str, owner_id: &str) -> crate::error::ToolError {
    crate::error::ToolError::AutonomousUnavailable {
        name: tool_name.to_string(),
        reason: autonomous_unavailable_message(tool_name, owner_id),
    }
}

pub async fn autonomous_allowed_tool_names(
    tools: &Arc<ToolRegistry>,
    extension_manager: Option<&Arc<ExtensionManager>>,
    owner_id: &str,
) -> HashSet<String> {
    let mut allowed = tools.builtin_tool_names().await;
    allowed.retain(|name| !is_autonomous_tool_denylisted(name));

    if let Some(extension_manager) = extension_manager
        && extension_manager.owner_id() == owner_id
    {
        allowed.extend(
            extension_manager
                .active_tool_names()
                .await
                .into_iter()
                .filter(|name| !is_autonomous_tool_denylisted(name)),
        );
    }

    allowed
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use async_trait::async_trait;
    use secrecy::SecretString;

    use super::*;
    use crate::context::JobContext;
    use crate::extensions::ExtensionManager;
    use crate::hooks::HookRegistry;
    use crate::secrets::{InMemorySecretsStore, SecretsCrypto, SecretsStore};
    use crate::tools::mcp::{McpProcessManager, McpSessionManager};
    use crate::tools::{Tool, ToolError, ToolOutput};

    struct FakeTool {
        name: &'static str,
    }

    #[async_trait]
    impl Tool for FakeTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "test tool"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {},
            })
        }

        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::text("ok", Duration::from_millis(1)))
        }
    }

    async fn write_test_extension_wasm(tools_dir: &Path, name: &str) {
        tokio::fs::create_dir_all(tools_dir)
            .await
            .expect("create test tools dir");
        tokio::fs::write(tools_dir.join(format!("{name}.wasm")), b"\0asm")
            .await
            .expect("write wasm marker");
    }

    fn make_extension_manager(
        tools: Arc<ToolRegistry>,
        tools_dir: &Path,
        owner_id: &str,
    ) -> Arc<ExtensionManager> {
        let crypto = Arc::new(
            SecretsCrypto::new(SecretString::from(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            ))
            .expect("test crypto"),
        );
        let secrets: Arc<dyn SecretsStore + Send + Sync> =
            Arc::new(InMemorySecretsStore::new(crypto));

        Arc::new(ExtensionManager::new(
            Arc::new(McpSessionManager::new()),
            Arc::new(McpProcessManager::new()),
            secrets,
            tools,
            Some(Arc::new(HookRegistry::default())),
            None,
            tools_dir.to_path_buf(),
            tools_dir.join("channels"),
            None,
            owner_id.to_string(),
            None,
            Vec::new(),
        ))
    }

    #[tokio::test]
    async fn autonomous_scope_keeps_allowed_builtins_and_blocks_denylisted_builtins() {
        let tools = Arc::new(ToolRegistry::new());
        tools.register_sync(Arc::new(FakeTool { name: "echo" }));
        tools.register_sync(Arc::new(FakeTool { name: "restart" }));

        let allowed = autonomous_allowed_tool_names(&tools, None, "default").await;

        assert!(allowed.contains("echo"));
        assert!(!allowed.contains("restart"));
    }

    #[tokio::test]
    async fn autonomous_scope_includes_active_extension_tools_for_matching_owner() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let tools_dir = temp_dir.path().join("wasm-tools");
        let tools = Arc::new(ToolRegistry::new());
        tools
            .register(Arc::new(FakeTool { name: "owner_gate" }))
            .await;
        write_test_extension_wasm(&tools_dir, "owner_gate").await;
        let manager = make_extension_manager(tools.clone(), &tools_dir, "default");

        let allowed = autonomous_allowed_tool_names(&tools, Some(&manager), "default").await;

        assert!(allowed.contains("owner_gate"));
    }

    #[tokio::test]
    async fn autonomous_scope_excludes_inactive_extension_tools() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let tools_dir = temp_dir.path().join("wasm-tools");
        let tools = Arc::new(ToolRegistry::new());
        let manager = make_extension_manager(tools.clone(), &tools_dir, "default");

        let allowed = autonomous_allowed_tool_names(&tools, Some(&manager), "default").await;

        assert!(!allowed.contains("owner_gate"));
    }

    #[tokio::test]
    async fn autonomous_scope_excludes_active_extension_tools_for_other_owner() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let tools_dir = temp_dir.path().join("wasm-tools");
        let tools = Arc::new(ToolRegistry::new());
        tools
            .register(Arc::new(FakeTool { name: "owner_gate" }))
            .await;
        write_test_extension_wasm(&tools_dir, "owner_gate").await;
        let manager = make_extension_manager(tools.clone(), &tools_dir, "someone-else");

        let allowed = autonomous_allowed_tool_names(&tools, Some(&manager), "default").await;

        assert!(!allowed.contains("owner_gate"));
    }
}
