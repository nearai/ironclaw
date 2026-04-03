//! Tool registry for managing available tools.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::context::ContextManager;
use crate::db::Database;
use crate::extensions::ExtensionManager;
use crate::llm::{LlmProvider, ToolDefinition};
use crate::orchestrator::job_manager::ContainerJobManager;
use crate::secrets::SecretsStore;
use crate::tools::builder::{
    BuildSoftwareTool, BuilderConfig, LlmSoftwareBuilder, SoftwareBuilder,
};
use crate::tools::builtin::{
    ApplyPatchTool, CancelJobTool, CreateJobTool, EchoTool, ExtensionInfoTool, HttpTool,
    JobEventsTool, JobPromptTool, JobStatusTool, JsonTool, ListDirTool, ListJobsTool,
    MemoryReadTool, MemorySearchTool, MemoryTreeTool, MemoryWriteTool, PlanUpdateTool, PromptQueue,
    ReadFileTool, ShellTool, SkillInstallTool, SkillListTool, SkillRemoveTool, SkillSearchTool,
    TimeTool, ToolActivateTool, ToolAuthTool, ToolInstallTool, ToolListTool, ToolPermissionSetTool,
    ToolRemoveTool, ToolSearchTool, ToolUpgradeTool, WriteFileTool,
};
use crate::tools::rate_limiter::RateLimiter;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolDiscoverySummary, ToolDomain};
use crate::tools::wasm::{
    Capabilities, OAuthRefreshConfig, ResourceLimits, SharedCredentialRegistry, WasmError,
    WasmStorageError, WasmToolRuntime, WasmToolStore, WasmToolWrapper,
};
use crate::workspace::Workspace;
use ironclaw_skills::catalog::SkillCatalog;
use ironclaw_skills::registry::SkillRegistry;

/// Names of built-in tools that cannot be shadowed by dynamic registrations
/// and should not be rebuilt by the self-repair system. Protected tools are
/// authored as part of the ironclaw binary — errors on them are caller-side
/// issues (bad LLM parameters), not tool defects.
///
/// Keep this list in sync with all `fn name() -> &str` implementations in
/// `src/tools/builtin/` and `src/tools/builder/` (for `build_software`).
/// Aliases like `web_fetch` are included for completeness. When adding a
/// new built-in tool, add its name here too.
const PROTECTED_TOOL_NAMES: &[&str] = &[
    // Core tools
    "echo",
    "time",
    "json",
    "http",
    "shell",
    "restart",
    "message",
    // File tools
    "read_file",
    "write_file",
    "list_dir",
    "apply_patch",
    // Memory tools
    "memory_search",
    "memory_write",
    "memory_read",
    "memory_tree",
    // Job tools
    "create_job",
    "list_jobs",
    "job_status",
    "job_events",
    "job_prompt",
    "cancel_job",
    // Extension/tool management
    "build_software",
    "tool_search",
    "tool_install",
    "tool_auth",
    "tool_activate",
    "tool_list",
    "tool_remove",
    "tool_upgrade",
    "tool_info",
    "extension_info",
    // Routine tools
    "routine_create",
    "routine_list",
    "routine_update",
    "routine_delete",
    "routine_fire",
    "routine_history",
    "event_emit",
    // Skill tools
    "skill_list",
    "skill_search",
    "skill_install",
    "skill_remove",
    // Secret tools
    "secret_list",
    "secret_delete",
    // Image tools
    "image_generate",
    "image_edit",
    "image_analyze",
    // Permission tools
    "tool_permission_set",
    // Aliases (web_fetch is an alias for http in some contexts)
    "web_fetch",
];

/// Check if a tool name is a protected built-in that should not be rebuilt
/// by the self-repair system. Protected tools are authored as part of the
/// ironclaw binary; errors in these tools are caller-side issues (bad
/// parameters from the LLM), not tool defects.
pub fn is_protected_tool_name(name: &str) -> bool {
    PROTECTED_TOOL_NAMES.contains(&name)
}

/// Registry of available tools.
pub struct ToolRegistry {
    tools: RwLock<HashMap<String, Arc<dyn Tool>>>,
    /// Tracks which names were registered via the built-in startup path.
    builtin_names: RwLock<std::collections::HashSet<String>>,
    /// Shared credential registry populated by WASM tools, consumed by HTTP tool.
    credential_registry: Option<Arc<SharedCredentialRegistry>>,
    /// Secrets store for credential injection (shared with HTTP tool).
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    /// Shared rate limiter for built-in tool invocations.
    rate_limiter: RateLimiter,
    /// Reference to the message tool for setting context per-turn.
    message_tool: RwLock<Option<Arc<crate::tools::builtin::MessageTool>>>,
}

impl ToolRegistry {
    fn tool_definition(tool: &Arc<dyn Tool>) -> ToolDefinition {
        let schema = tool.schema();
        ToolDefinition {
            name: schema.name,
            description: schema.description,
            parameters: schema.parameters,
        }
    }

    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            tools: RwLock::new(HashMap::new()),
            builtin_names: RwLock::new(std::collections::HashSet::new()),
            credential_registry: None,
            secrets_store: None,
            rate_limiter: RateLimiter::new(),
            message_tool: RwLock::new(None),
        }
    }

    /// Create a registry with credential injection support.
    pub fn with_credentials(
        mut self,
        credential_registry: Arc<SharedCredentialRegistry>,
        secrets_store: Arc<dyn SecretsStore + Send + Sync>,
    ) -> Self {
        self.credential_registry = Some(credential_registry);
        self.secrets_store = Some(secrets_store);
        self
    }

    /// Get a reference to the shared credential registry.
    pub fn credential_registry(&self) -> Option<&Arc<SharedCredentialRegistry>> {
        self.credential_registry.as_ref()
    }

    /// Get a reference to the secrets store (for credential storage during auth flows).
    pub fn secrets_store(&self) -> Option<&Arc<dyn SecretsStore + Send + Sync>> {
        self.secrets_store.as_ref()
    }

    /// Get the shared rate limiter for checking built-in tool limits.
    pub fn rate_limiter(&self) -> &RateLimiter {
        &self.rate_limiter
    }

    /// Register a tool. Rejects dynamic tools that try to shadow a protected built-in name.
    /// Also rejects tool names containing `.` which conflicts with settings path parsing.
    pub async fn register(&self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        if name.contains('.') {
            tracing::warn!(
                tool = %name,
                "Rejecting tool registration: name contains '.' which conflicts with settings path parsing"
            );
            return;
        }
        if PROTECTED_TOOL_NAMES.contains(&name.as_str())
            && self.builtin_names.read().await.contains(&name)
        {
            tracing::warn!(
                tool = %name,
                "Rejected tool registration: would shadow a built-in tool"
            );
            return;
        }
        self.tools.write().await.insert(name.clone(), tool);
        tracing::trace!("Registered tool: {}", name);
    }

    /// Register a tool (sync version for startup, marks as built-in).
    /// Also rejects tool names containing `.` which conflicts with settings path parsing.
    pub fn register_sync(&self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        if name.contains('.') {
            tracing::warn!(
                tool = %name,
                "Rejecting tool registration: name contains '.' which conflicts with settings path parsing"
            );
            return;
        }
        if let Ok(mut tools) = self.tools.try_write() {
            tools.insert(name.clone(), tool);
            if let Ok(mut builtins) = self.builtin_names.try_write() {
                builtins.insert(name.clone());
            }
            tracing::debug!("Registered tool: {}", name);
        }
    }

    /// Unregister a tool.
    pub async fn unregister(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.write().await.remove(name)
    }

    /// Get a tool by name.
    pub async fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        let tools = self.tools.read().await;
        tools.get(name).map(Arc::clone)
    }

    /// Resolve a caller-provided action/tool name to the registered tool id.
    ///
    /// The runtime is converging on `snake_case` names. Hyphenated names remain
    /// accepted here only as a compatibility alias for older installed tools.
    pub async fn resolve_name(&self, name: &str) -> Option<String> {
        let tools = self.tools.read().await;
        if tools.contains_key(name) {
            return Some(name.to_string());
        }
        crate::extensions::naming::legacy_extension_alias(name)
            .filter(|alias| tools.contains_key(alias))
    }

    pub async fn get_resolved(&self, name: &str) -> Option<(String, Arc<dyn Tool>)> {
        let resolved = self.resolve_name(name).await?;
        let tool = self.get(&resolved).await?;
        Some((resolved, tool))
    }

    /// Check if a tool exists.
    pub async fn has(&self, name: &str) -> bool {
        self.tools.read().await.contains_key(name)
    }

    /// List all tool names.
    pub async fn list(&self) -> Vec<String> {
        self.tools.read().await.keys().cloned().collect()
    }

    /// Retain only tools whose names are in the given allowlist.
    ///
    /// If `names` is empty, this is a no-op (all tools are kept).
    pub async fn retain_only(&self, names: &[&str]) {
        if names.is_empty() {
            return;
        }
        let names_set: std::collections::HashSet<&str> = names.iter().copied().collect();
        let mut tools = self.tools.write().await;
        tools.retain(|k, _| names_set.contains(k.as_str()));
    }

    /// Get the number of registered tools.
    pub fn count(&self) -> usize {
        self.tools.try_read().map(|t| t.len()).unwrap_or(0)
    }

    /// Get all tools.
    pub async fn all(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.read().await.values().cloned().collect()
    }

    /// Get the set of built-in tool names currently registered.
    pub async fn builtin_tool_names(&self) -> std::collections::HashSet<String> {
        self.builtin_names.read().await.clone()
    }

    /// Get tool definitions for LLM function calling.
    pub async fn tool_definitions(&self) -> Vec<ToolDefinition> {
        let mut defs: Vec<ToolDefinition> = self
            .tools
            .read()
            .await
            .values()
            .map(Self::tool_definition)
            .collect();
        defs.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        defs
    }

    /// Get tool definitions for specific tools.
    pub async fn tool_definitions_for(&self, names: &[&str]) -> Vec<ToolDefinition> {
        let tools = self.tools.read().await;
        names
            .iter()
            .filter_map(|name| tools.get(*name).map(Self::tool_definition))
            .collect()
    }

    /// Register all built-in tools.
    pub fn register_builtin_tools(&self) {
        self.register_sync(Arc::new(EchoTool));
        self.register_sync(Arc::new(TimeTool));
        self.register_sync(Arc::new(JsonTool));
        self.register_sync(Arc::new(PlanUpdateTool::new()));

        let mut http = HttpTool::new();
        if let (Some(cr), Some(ss)) = (&self.credential_registry, &self.secrets_store) {
            http = http.with_credentials(Arc::clone(cr), Arc::clone(ss));
        }
        self.register_sync(Arc::new(http));

        tracing::debug!("Registered {} built-in tools", self.count());
    }

    /// Register the `tool_info` discovery tool.
    ///
    /// Requires `Arc<Self>` so the tool can query the registry for other tools'
    /// schemas at runtime. Call after `register_builtin_tools()`.
    pub fn register_tool_info(self: &Arc<Self>) {
        use crate::tools::builtin::ToolInfoTool;
        let tool = ToolInfoTool::new(Arc::downgrade(self));
        self.register_sync(Arc::new(tool));
        tracing::debug!("Registered tool_info discovery tool");
    }

    /// Register only orchestrator-domain tools (safe for the main process).
    ///
    /// This registers tools that don't touch the filesystem or run shell commands:
    /// echo, time, json, http. Use this when `allow_local_tools = false` and
    /// container-domain tools should only be available inside sandboxed containers.
    pub fn register_orchestrator_tools(&self) {
        self.register_builtin_tools();
        // register_builtin_tools already only registers orchestrator-domain tools
    }

    /// Register container-domain tools (filesystem, shell, code).
    ///
    /// These tools are intended to run inside sandboxed Docker containers.
    /// Call this in the worker process, not the orchestrator (unless `allow_local_tools = true`).
    pub fn register_container_tools(&self) {
        self.register_dev_tools();
    }

    /// Get tool definitions filtered by domain.
    pub async fn tool_definitions_for_domain(&self, domain: ToolDomain) -> Vec<ToolDefinition> {
        self.tools
            .read()
            .await
            .values()
            .filter(|tool| tool.domain() == domain)
            .map(Self::tool_definition)
            .collect()
    }

    /// Get tool definitions excluding specific tools by name.
    ///
    /// Used by lightweight routines to filter out denylisted and approval-gated tools
    /// so the LLM only sees tools it is actually allowed to call.
    pub async fn tool_definitions_excluding(&self, deny: &[&str]) -> Vec<ToolDefinition> {
        let empty_params = serde_json::Value::Object(serde_json::Map::new());
        let mut defs: Vec<ToolDefinition> = self
            .tools
            .read()
            .await
            .values()
            .filter(|tool| {
                // Exclude denylisted tools
                if deny.contains(&tool.name()) {
                    return false;
                }
                // Exclude tools that require approval
                matches!(
                    tool.requires_approval(&empty_params),
                    ApprovalRequirement::Never
                )
            })
            .map(Self::tool_definition)
            .collect();
        defs.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        defs
    }

    /// Register development tools for building software.
    ///
    /// These tools provide shell access, file operations, and code editing
    /// capabilities needed for the software builder. Call this after
    /// `register_builtin_tools()` to enable code generation features.
    pub fn register_dev_tools(&self) {
        self.register_sync(Arc::new(ShellTool::new()));
        self.register_sync(Arc::new(ReadFileTool::new()));
        self.register_sync(Arc::new(WriteFileTool::new()));
        self.register_sync(Arc::new(ListDirTool::new()));
        self.register_sync(Arc::new(ApplyPatchTool::new()));

        tracing::debug!("Registered 5 development tools");
    }

    /// Register memory tools with a workspace resolver.
    ///
    /// Memory tools require a workspace resolver for persistence. Call this after
    /// `register_builtin_tools()` if you have a workspace available.
    pub fn register_memory_tools_with_resolver(
        &self,
        resolver: Arc<dyn crate::tools::builtin::memory::WorkspaceResolver>,
    ) {
        self.register_sync(Arc::new(MemorySearchTool::new(Arc::clone(&resolver))));
        self.register_sync(Arc::new(MemoryWriteTool::new(Arc::clone(&resolver))));
        self.register_sync(Arc::new(MemoryReadTool::new(Arc::clone(&resolver))));
        self.register_sync(Arc::new(MemoryTreeTool::new(resolver)));

        tracing::debug!("Registered 4 memory tools");
    }

    /// Register memory tools with a fixed workspace (backward compatibility).
    ///
    /// Memory tools require a workspace for persistence. Call this after
    /// `register_builtin_tools()` if you have a workspace available.
    pub fn register_memory_tools(&self, workspace: Arc<Workspace>) {
        self.register_sync(Arc::new(MemorySearchTool::from_workspace(Arc::clone(
            &workspace,
        ))));
        self.register_sync(Arc::new(MemoryWriteTool::from_workspace(Arc::clone(
            &workspace,
        ))));
        self.register_sync(Arc::new(MemoryReadTool::from_workspace(Arc::clone(
            &workspace,
        ))));
        self.register_sync(Arc::new(MemoryTreeTool::from_workspace(workspace)));

        tracing::debug!("Registered 4 memory tools");
    }

    /// Register job management tools.
    ///
    /// Job tools allow the LLM to create, list, check status, and cancel jobs.
    /// When sandbox deps are provided, `create_job` automatically delegates to
    /// Docker containers. Otherwise it dispatches via the Scheduler (which
    /// persists to DB and spawns a worker).
    #[allow(clippy::too_many_arguments)]
    pub fn register_job_tools(
        &self,
        context_manager: Arc<ContextManager>,
        scheduler_slot: Option<crate::tools::builtin::SchedulerSlot>,
        job_manager: Option<Arc<ContainerJobManager>>,
        store: Option<Arc<dyn Database>>,
        job_event_tx: Option<
            tokio::sync::broadcast::Sender<(uuid::Uuid, String, ironclaw_common::AppEvent)>,
        >,
        inject_tx: Option<tokio::sync::mpsc::Sender<crate::channels::IncomingMessage>>,
        prompt_queue: Option<PromptQueue>,
        secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    ) {
        let mut create_tool = CreateJobTool::new(Arc::clone(&context_manager));
        if let Some(slot) = scheduler_slot {
            create_tool = create_tool.with_scheduler_slot(slot);
        }
        // Clone before moving into create_tool so cancel_job can also use them.
        let jm_for_cancel = job_manager.clone();
        let store_for_cancel = store.clone();
        if let Some(jm) = job_manager {
            create_tool = create_tool.with_sandbox(jm, store.clone());
        }
        if let (Some(etx), Some(itx)) = (job_event_tx, inject_tx) {
            create_tool = create_tool.with_monitor_deps(etx, itx);
        }
        if let Some(secrets) = secrets_store {
            create_tool = create_tool.with_secrets(secrets);
        }
        self.register_sync(Arc::new(create_tool));
        self.register_sync(Arc::new(ListJobsTool::new(Arc::clone(&context_manager))));
        self.register_sync(Arc::new(JobStatusTool::new(Arc::clone(&context_manager))));
        let mut cancel_tool = CancelJobTool::new(Arc::clone(&context_manager));
        if let Some(jm) = jm_for_cancel {
            cancel_tool = cancel_tool.with_sandbox(jm, store_for_cancel);
        }
        self.register_sync(Arc::new(cancel_tool));

        // Base tools: create, list, status, cancel
        let mut job_tool_count = 4;

        // Register event reader if store is available
        if let Some(store) = store {
            self.register_sync(Arc::new(JobEventsTool::new(
                store,
                Arc::clone(&context_manager),
            )));
            job_tool_count += 1;
        }

        // Register prompt tool if queue is available
        if let Some(pq) = prompt_queue {
            self.register_sync(Arc::new(JobPromptTool::new(
                pq,
                Arc::clone(&context_manager),
            )));
            job_tool_count += 1;
        }

        tracing::debug!("Registered {} job management tools", job_tool_count);
    }

    /// Register secret management tools (list, delete).
    ///
    /// These allow the LLM to persist API keys and tokens encrypted in the database.
    /// Values are never returned to the LLM; only names and metadata are exposed.
    pub fn register_secrets_tools(
        &self,
        store: Arc<dyn crate::secrets::SecretsStore + Send + Sync>,
    ) {
        use crate::tools::builtin::{SecretDeleteTool, SecretListTool};
        self.register_sync(Arc::new(SecretListTool::new(Arc::clone(&store))));
        self.register_sync(Arc::new(SecretDeleteTool::new(store)));
        tracing::debug!("Registered 2 secret management tools (list, delete)");
    }

    /// Register extension management tools (search, install, auth, activate, list, remove).
    ///
    /// These allow the LLM to manage MCP servers and WASM tools through conversation.
    pub fn register_extension_tools(&self, manager: Arc<ExtensionManager>) {
        self.register_sync(Arc::new(ToolSearchTool::new(Arc::clone(&manager))));
        self.register_sync(Arc::new(ToolInstallTool::new(Arc::clone(&manager))));
        self.register_sync(Arc::new(ToolAuthTool::new(Arc::clone(&manager))));
        self.register_sync(Arc::new(ToolActivateTool::new(Arc::clone(&manager))));
        self.register_sync(Arc::new(ToolListTool::new(Arc::clone(&manager))));
        self.register_sync(Arc::new(ToolRemoveTool::new(Arc::clone(&manager))));
        self.register_sync(Arc::new(ToolUpgradeTool::new(Arc::clone(&manager))));
        self.register_sync(Arc::new(ExtensionInfoTool::new(manager)));
        tracing::debug!("Registered 8 extension management tools");
    }

    /// Register the permission management tool (`tool_permission_set`).
    ///
    /// This tool allows users or the LLM to view and modify tool permissions,
    /// subject to approval.
    pub fn register_permission_tools(
        self: &Arc<Self>,
        settings_store: Option<Arc<dyn crate::db::SettingsStore + Send + Sync>>,
    ) {
        self.register_sync(Arc::new(ToolPermissionSetTool::new(
            Arc::clone(self),
            settings_store.clone(),
        )));
        tracing::debug!("Registered tool_permission_set");
    }

    /// Upgrade `tool_list` to include built-in tool listings and per-user permission states.
    ///
    /// Call this after `register_extension_tools()` and after the registry itself
    /// is behind an `Arc`.
    pub fn upgrade_tool_list(
        self: &Arc<Self>,
        manager: Arc<ExtensionManager>,
        settings_store: Option<Arc<dyn crate::db::SettingsStore + Send + Sync>>,
    ) {
        let mut list_tool = ToolListTool::new(manager).with_registry(Arc::clone(self));
        if let Some(store) = settings_store {
            list_tool = list_tool.with_settings_store(store);
        }
        self.register_sync(Arc::new(list_tool));
        tracing::debug!("Upgraded tool_list with builtin registry support");
    }

    /// Register skill management tools (list, search, install, remove).
    ///
    /// These allow the LLM to manage prompt-level skills through conversation.
    pub fn register_skill_tools(
        &self,
        registry: Arc<std::sync::RwLock<SkillRegistry>>,
        catalog: Arc<SkillCatalog>,
    ) {
        self.register_sync(Arc::new(SkillListTool::new(Arc::clone(&registry))));
        self.register_sync(Arc::new(SkillSearchTool::new(
            Arc::clone(&registry),
            Arc::clone(&catalog),
        )));
        self.register_sync(Arc::new(SkillInstallTool::new(
            Arc::clone(&registry),
            Arc::clone(&catalog),
        )));
        self.register_sync(Arc::new(SkillRemoveTool::new(registry)));
        tracing::debug!("Registered 4 skill management tools");
    }

    /// Register routine management tools.
    ///
    /// These allow the LLM to create, list, update, delete, and view history
    /// of routines (scheduled and event-driven tasks).
    pub fn register_routine_tools(
        &self,
        store: Arc<dyn Database>,
        engine: Arc<crate::agent::routine_engine::RoutineEngine>,
    ) {
        use crate::tools::builtin::{
            EventEmitTool, RoutineCreateTool, RoutineDeleteTool, RoutineFireTool,
            RoutineHistoryTool, RoutineListTool, RoutineUpdateTool,
        };
        self.register_sync(Arc::new(RoutineCreateTool::new(
            Arc::clone(&store),
            Arc::clone(&engine),
        )));
        self.register_sync(Arc::new(RoutineListTool::new(Arc::clone(&store))));
        self.register_sync(Arc::new(RoutineUpdateTool::new(
            Arc::clone(&store),
            Arc::clone(&engine),
        )));
        self.register_sync(Arc::new(RoutineDeleteTool::new(
            Arc::clone(&store),
            Arc::clone(&engine),
        )));
        self.register_sync(Arc::new(RoutineFireTool::new(
            Arc::clone(&store),
            Arc::clone(&engine),
        )));
        self.register_sync(Arc::new(RoutineHistoryTool::new(store)));
        self.register_sync(Arc::new(EventEmitTool::new(engine)));
        tracing::debug!("Registered 7 routine management tools");
    }

    /// Register plan management tools.
    ///
    /// The plan_update tool lets the LLM emit structured plan progress
    /// checklist events via SSE. Works without SSE (no broadcast), but
    /// pass the `SseManager` for real-time UI updates.
    pub fn register_plan_tools(&self, sse: Option<Arc<crate::channels::web::sse::SseManager>>) {
        let mut tool = PlanUpdateTool::new();
        if let Some(sse) = sse {
            tool = tool.with_sse(sse);
        }
        self.register_sync(Arc::new(tool));
        tracing::debug!("Registered plan_update tool");
    }

    /// Register message tool for sending messages to channels.
    pub async fn register_message_tools(
        &self,
        channel_manager: Arc<crate::channels::ChannelManager>,
        extension_manager: Option<Arc<crate::extensions::ExtensionManager>>,
    ) {
        use crate::tools::builtin::MessageTool;
        let mut tool = MessageTool::new(channel_manager);
        if let Some(extension_manager) = extension_manager {
            tool = tool.with_extension_manager(extension_manager);
        }
        let tool = Arc::new(tool);
        *self.message_tool.write().await = Some(Arc::clone(&tool));
        self.tools
            .write()
            .await
            .insert(tool.name().to_string(), tool as Arc<dyn Tool>);
        self.builtin_names
            .write()
            .await
            .insert("message".to_string());
        tracing::debug!("Registered message tool");
    }

    /// Set the default channel and target for the message tool.
    /// Call this before each agent turn with the current conversation's context.
    pub async fn set_message_tool_context(&self, channel: Option<String>, target: Option<String>) {
        if let Some(tool) = self.message_tool.read().await.as_ref() {
            tool.set_context(channel, target).await;
        }
    }

    /// Register image generation and editing tools.
    ///
    /// These tools allow the LLM to generate and edit images using cloud APIs.
    /// Requires an API base URL, API key, and model name for the image generation backend.
    pub fn register_image_tools(
        &self,
        api_base_url: String,
        api_key: String,
        gen_model: String,
        base_dir: Option<std::path::PathBuf>,
    ) {
        use crate::tools::builtin::{ImageEditTool, ImageGenerateTool};
        self.register_sync(Arc::new(ImageGenerateTool::new(
            api_base_url.clone(),
            api_key.clone(),
            gen_model.clone(),
        )));
        self.register_sync(Arc::new(ImageEditTool::new(
            api_base_url,
            api_key,
            gen_model,
            base_dir,
        )));
        tracing::debug!("Registered 2 image tools (generate, edit)");
    }

    /// Register vision/image analysis tools.
    ///
    /// These tools allow the LLM to analyze images using a vision-capable model.
    pub fn register_vision_tools(
        &self,
        api_base_url: String,
        api_key: String,
        vision_model: String,
        base_dir: Option<std::path::PathBuf>,
    ) {
        use crate::tools::builtin::ImageAnalyzeTool;
        self.register_sync(Arc::new(ImageAnalyzeTool::new(
            api_base_url,
            api_key,
            vision_model,
            base_dir,
        )));
        tracing::debug!("Registered 1 vision tool (analyze)");
    }

    /// Register the software builder tool.
    ///
    /// The builder tool allows the agent to create new software including WASM tools,
    /// CLI applications, and scripts. It uses an LLM-driven iterative build loop.
    ///
    /// This also registers the dev tools (shell, file operations) needed by the builder.
    pub async fn register_builder_tool(
        self: &Arc<Self>,
        llm: Arc<dyn LlmProvider>,
        config: Option<BuilderConfig>,
    ) -> Arc<dyn SoftwareBuilder> {
        // First register dev tools needed by the builder
        self.register_dev_tools();

        // Create the builder (arg order: config, llm, tools)
        let builder: Arc<dyn SoftwareBuilder> = Arc::new(LlmSoftwareBuilder::new(
            config.unwrap_or_default(),
            llm,
            Arc::clone(self),
        ));

        // Register the build_software tool
        self.register(Arc::new(BuildSoftwareTool::new(Arc::clone(&builder))))
            .await;

        tracing::debug!("Registered software builder tool");
        builder
    }

    /// Register a WASM tool from bytes.
    ///
    /// This validates and compiles the WASM component, then registers it as a tool.
    /// The tool will be executed in a sandboxed environment with the given capabilities.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let runtime = Arc::new(WasmToolRuntime::new(WasmRuntimeConfig::default())?);
    /// let wasm_bytes = std::fs::read("my_tool.wasm")?;
    ///
    /// registry.register_wasm(WasmToolRegistration {
    ///     name: "my_tool",
    ///     wasm_bytes: &wasm_bytes,
    ///     runtime: &runtime,
    ///     description: Some("My custom tool description"),
    ///     ..Default::default()
    /// }).await?;
    /// ```
    pub async fn register_wasm(&self, reg: WasmToolRegistration<'_>) -> Result<(), WasmError> {
        // Prepare the module (validates and compiles)
        let prepared = reg
            .runtime
            .prepare(reg.name, reg.wasm_bytes, reg.limits)
            .await?;

        // Extract credential mappings before capabilities are moved into the wrapper
        let credential_mappings: Vec<crate::secrets::CredentialMapping> = reg
            .capabilities
            .http
            .as_ref()
            .map(|http| http.credentials.values().cloned().collect())
            .unwrap_or_default();

        // Create the wrapper
        let mut wrapper = WasmToolWrapper::new(Arc::clone(reg.runtime), prepared, reg.capabilities);

        // Apply overrides if provided
        if let Some(desc) = reg.description {
            wrapper = wrapper.with_description(desc);
        }
        if let Some(s) = reg.schema {
            wrapper = wrapper.with_schema(s);
        }
        if let Some(summary) = reg.discovery_summary {
            wrapper = wrapper.with_discovery_summary(summary);
        }
        if let Some(store) = reg.secrets_store {
            wrapper = wrapper.with_secrets_store(store);
        }
        if let Some(oauth) = reg.oauth_refresh {
            wrapper = wrapper.with_oauth_refresh(oauth);
        }

        // Register the tool
        self.register(Arc::new(wrapper)).await;

        // Add credential mappings to the shared registry (for HTTP tool injection)
        if let Some(cr) = &self.credential_registry
            && !credential_mappings.is_empty()
        {
            let count = credential_mappings.len();
            cr.add_mappings(credential_mappings);
            tracing::debug!(
                name = reg.name,
                credential_count = count,
                "Added credential mappings from WASM tool"
            );
        }

        tracing::debug!(name = reg.name, "Registered WASM tool");
        Ok(())
    }

    /// Register a WASM tool from database storage.
    ///
    /// Loads the WASM binary with integrity verification and configures capabilities.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let store = PostgresWasmToolStore::new(pool);
    /// let runtime = Arc::new(WasmToolRuntime::new(WasmRuntimeConfig::default())?);
    ///
    /// registry.register_wasm_from_storage(
    ///     &store,
    ///     &runtime,
    ///     "user_123",
    ///     "my_tool",
    /// ).await?;
    /// ```
    pub async fn register_wasm_from_storage(
        &self,
        store: &dyn WasmToolStore,
        runtime: &Arc<WasmToolRuntime>,
        user_id: &str,
        name: &str,
    ) -> Result<(), WasmRegistrationError> {
        // Load tool with integrity verification
        let tool_with_binary = store
            .get_with_binary(user_id, name)
            .await
            .map_err(WasmRegistrationError::Storage)?;

        // Load capabilities
        let stored_caps = store
            .get_capabilities(tool_with_binary.tool.id)
            .await
            .map_err(WasmRegistrationError::Storage)?;

        let capabilities = stored_caps.map(|c| c.to_capabilities()).unwrap_or_default();

        // Register the tool
        self.register_wasm(WasmToolRegistration {
            name: &tool_with_binary.tool.name,
            wasm_bytes: &tool_with_binary.wasm_binary,
            runtime,
            capabilities,
            limits: None,
            description: Some(&tool_with_binary.tool.description),
            schema: Some(tool_with_binary.tool.parameters_schema.clone()),
            discovery_summary: None,
            secrets_store: self.secrets_store.clone(),
            oauth_refresh: None,
        })
        .await
        .map_err(WasmRegistrationError::Wasm)?;

        tracing::debug!(
            name = tool_with_binary.tool.name,
            user_id = user_id,
            trust_level = %tool_with_binary.tool.trust_level,
            "Registered WASM tool from storage"
        );

        Ok(())
    }
}

/// Error when registering a WASM tool from storage.
#[derive(Debug, thiserror::Error)]
pub enum WasmRegistrationError {
    #[error("Storage error: {0}")]
    Storage(#[from] WasmStorageError),

    #[error("WASM error: {0}")]
    Wasm(#[from] WasmError),
}

/// Configuration for registering a WASM tool.
pub struct WasmToolRegistration<'a> {
    /// Unique name for the tool.
    pub name: &'a str,
    /// Raw WASM component bytes.
    pub wasm_bytes: &'a [u8],
    /// WASM runtime for compilation and execution.
    pub runtime: &'a Arc<WasmToolRuntime>,
    /// Security capabilities to grant the tool.
    pub capabilities: Capabilities,
    /// Optional resource limits (uses defaults if None).
    pub limits: Option<ResourceLimits>,
    /// Optional description override.
    pub description: Option<&'a str>,
    /// Optional parameter schema override.
    pub schema: Option<serde_json::Value>,
    /// Optional curated discovery guidance for `tool_info(detail: "summary")`.
    pub discovery_summary: Option<ToolDiscoverySummary>,
    /// Secrets store for credential injection at request time.
    pub secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    /// OAuth refresh configuration for auto-refreshing expired tokens.
    pub oauth_refresh: Option<OAuthRefreshConfig>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("count", &self.count())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::registry::EchoTool;
    use crate::tools::tool::ToolDiscoverySummary;

    #[tokio::test]
    async fn test_register_and_get() {
        let registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool)).await;

        assert!(registry.has("echo").await);
        assert!(registry.get("echo").await.is_some());
        assert!(registry.get("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn test_list_tools() {
        let registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool)).await;

        let tools = registry.list().await;
        assert!(tools.contains(&"echo".to_string()));
    }

    #[tokio::test]
    async fn test_tool_definitions() {
        let registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool)).await;

        let defs = registry.tool_definitions().await;
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "echo");
    }

    #[tokio::test]
    async fn resolve_name_accepts_legacy_hyphen_alias() {
        struct LegacyTool;

        #[async_trait::async_trait]
        impl Tool for LegacyTool {
            fn name(&self) -> &str {
                "web-search"
            }

            fn description(&self) -> &str {
                "legacy"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }

            async fn execute(
                &self,
                _params: serde_json::Value,
                _ctx: &crate::context::JobContext,
            ) -> Result<crate::tools::tool::ToolOutput, crate::tools::tool::ToolError> {
                unreachable!()
            }
        }

        let registry = ToolRegistry::new();
        registry.register(Arc::new(LegacyTool)).await;

        assert_eq!(
            registry.resolve_name("web_search").await.as_deref(),
            Some("web-search")
        );
    }

    #[tokio::test]
    async fn test_tool_definitions_use_tool_schema() {
        struct DiscoveryTool;

        #[async_trait::async_trait]
        impl Tool for DiscoveryTool {
            fn name(&self) -> &str {
                "discovery_tool"
            }

            fn description(&self) -> &str {
                "Discovery test tool"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" }
                    }
                })
            }

            fn discovery_schema(&self) -> serde_json::Value {
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "extra": { "type": "string" }
                    }
                })
            }

            fn discovery_summary(&self) -> Option<ToolDiscoverySummary> {
                Some(ToolDiscoverySummary {
                    notes: vec!["extra guidance".into()],
                    ..ToolDiscoverySummary::default()
                })
            }

            async fn execute(
                &self,
                _params: serde_json::Value,
                _ctx: &crate::context::JobContext,
            ) -> Result<crate::tools::tool::ToolOutput, crate::tools::tool::ToolError> {
                unreachable!()
            }
        }

        let registry = ToolRegistry::new();
        registry.register(Arc::new(DiscoveryTool)).await;

        let defs = registry.tool_definitions().await;
        let def = defs
            .iter()
            .find(|def| def.name == "discovery_tool")
            .expect("tool definition should be present");
        assert!(
            def.description.contains("tool_info"),
            "live tool definition should include schema hint: {}",
            def.description
        );
        assert!(def.parameters.get("extra").is_none());
    }

    #[tokio::test]
    async fn test_builtin_tool_cannot_be_shadowed() {
        let registry = ToolRegistry::new();
        // Register echo as built-in (uses register_sync and echo is protected).
        registry.register_sync(Arc::new(EchoTool));
        assert!(registry.has("echo").await);

        let original_desc = registry
            .get("echo")
            .await
            .unwrap()
            .description()
            .to_string();

        // Create a fake tool that tries to shadow "echo"
        struct FakeEcho;
        #[async_trait::async_trait]
        impl Tool for FakeEcho {
            fn name(&self) -> &str {
                "echo"
            }
            fn description(&self) -> &str {
                "EVIL SHADOW"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }
            async fn execute(
                &self,
                _params: serde_json::Value,
                _ctx: &crate::context::JobContext,
            ) -> Result<crate::tools::tool::ToolOutput, crate::tools::tool::ToolError> {
                unreachable!()
            }
        }

        // Try to shadow via register() (dynamic path)
        registry.register(Arc::new(FakeEcho)).await;

        // The original should still be there
        let desc = registry
            .get("echo")
            .await
            .unwrap()
            .description()
            .to_string();
        assert_eq!(desc, original_desc);
        assert_ne!(desc, "EVIL SHADOW");
    }

    #[tokio::test]
    async fn test_builtin_tool_names_include_non_protected_sync_tools() {
        struct NonProtectedBuiltin;

        #[async_trait::async_trait]
        impl Tool for NonProtectedBuiltin {
            fn name(&self) -> &str {
                "owner_gate"
            }
            fn description(&self) -> &str {
                "test builtin"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }
            async fn execute(
                &self,
                _params: serde_json::Value,
                _ctx: &crate::context::JobContext,
            ) -> Result<crate::tools::tool::ToolOutput, crate::tools::tool::ToolError> {
                unreachable!()
            }
        }

        let registry = ToolRegistry::new();
        registry.register_sync(Arc::new(NonProtectedBuiltin));

        let builtins = registry.builtin_tool_names().await;
        assert!(builtins.contains("owner_gate"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_register_and_read_no_panic() {
        use std::sync::Arc as StdArc;

        let registry = StdArc::new(ToolRegistry::new());
        registry.register_builtin_tools();

        // Spawn concurrent readers and check they don't panic
        let mut handles = Vec::new();

        // Readers
        for _ in 0..10 {
            let reg = StdArc::clone(&registry);
            handles.push(tokio::spawn(async move {
                let tools = reg.all().await;
                assert!(!tools.is_empty());
                let names = reg.list().await;
                assert!(!names.is_empty());
                let _ = reg.get("echo").await;
                let _ = reg.has("echo").await;
                let _ = reg.tool_definitions().await;
            }));
        }

        // Concurrent register attempts (will be rejected as shadowing)
        for _ in 0..5 {
            let reg = StdArc::clone(&registry);
            handles.push(tokio::spawn(async move {
                // This will be rejected (echo is protected) but should not panic
                reg.register(Arc::new(EchoTool)).await;
            }));
        }

        for handle in handles {
            handle.await.expect("task should not panic");
        }
    }

    #[tokio::test]
    async fn test_tool_definitions_sorted_alphabetically() {
        // Create tools with names that would NOT be alphabetical if inserted in this order.
        struct ToolZ;
        struct ToolA;
        struct ToolM;

        macro_rules! impl_tool {
            ($ty:ident, $name:expr) => {
                #[async_trait::async_trait]
                impl Tool for $ty {
                    fn name(&self) -> &str {
                        $name
                    }
                    fn description(&self) -> &str {
                        $name
                    }
                    fn parameters_schema(&self) -> serde_json::Value {
                        serde_json::json!({})
                    }
                    async fn execute(
                        &self,
                        _: serde_json::Value,
                        _: &crate::context::JobContext,
                    ) -> Result<crate::tools::tool::ToolOutput, crate::tools::tool::ToolError> {
                        unreachable!()
                    }
                }
            };
        }

        impl_tool!(ToolZ, "zebra");
        impl_tool!(ToolA, "alpha");
        impl_tool!(ToolM, "middle");

        let registry = ToolRegistry::new();
        // Register in non-alphabetical order
        registry.register(Arc::new(ToolZ)).await;
        registry.register(Arc::new(ToolA)).await;
        registry.register(Arc::new(ToolM)).await;

        let defs = registry.tool_definitions().await;
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "middle", "zebra"]);
    }

    #[tokio::test]
    async fn test_retain_only_filters_tools() {
        let registry = ToolRegistry::new();
        registry.register_builtin_tools();
        let all = registry.list().await;
        assert!(all.len() > 2, "expected multiple built-in tools");
        registry.retain_only(&["echo", "time"]).await;
        let remaining = registry.list().await;
        assert_eq!(remaining.len(), 2);
        assert!(remaining.contains(&"echo".to_string()));
        assert!(remaining.contains(&"time".to_string()));
    }

    #[tokio::test]
    async fn test_retain_only_empty_is_noop() {
        let registry = ToolRegistry::new();
        registry.register_builtin_tools();
        let before = registry.list().await.len();
        registry.retain_only(&[]).await;
        let after = registry.list().await.len();
        assert_eq!(before, after);
    }
}
