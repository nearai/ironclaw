//! Tool registry for managing available tools.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::context::ContextManager;
use crate::db::{Database, UserStore};
use crate::extensions::ExtensionManager;
use crate::llm::recording::HttpInterceptor;
use crate::llm::{LlmProvider, ToolDefinition};
use crate::orchestrator::job_manager::ContainerJobManager;
use crate::secrets::SecretsStore;
use crate::tools::builder::{
    BuildSoftwareTool, BuilderConfig, LlmSoftwareBuilder, SoftwareBuilder,
};
use crate::tools::builtin::{
    ApplyPatchTool, CancelJobTool, CreateJobTool, EchoTool, ExtensionInfoTool, FileUndoTool,
    GlobTool, GrepTool, HttpTool, JobEventsTool, JobPromptTool, JobStatusTool, JsonTool,
    ListDirTool, ListJobsTool, MemoryReadTool, MemorySearchTool, MemoryTreeTool, MemoryWriteTool,
    PlanUpdateTool, PromptQueue, ReadFileTool, ShellTool, SkillInstallTool, SkillListTool,
    SkillRemoveTool, SkillSearchTool, TimeTool, ToolActivateTool, ToolAuthTool, ToolInstallTool,
    ToolListTool, ToolPermissionSetTool, ToolRemoveTool, ToolSearchTool, ToolUpgradeTool,
    WriteFileTool, shared_file_history, shared_read_file_state,
};
use crate::tools::rate_limiter::RateLimiter;
use crate::tools::tool::{
    ApprovalRequirement, EngineVersion, Tool, ToolDiscoverySummary, ToolDomain,
};
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
    "glob",
    "grep",
    "file_undo",
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
    // Plan tools
    "plan_update",
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
    /// Narrow role lookup used by runtime credential fallback.
    role_lookup: Option<Arc<dyn UserStore>>,
    /// Database handle for user-role checks in multi-tenant credential fallback.
    db: Option<Arc<dyn Database>>,
    /// Shared rate limiter for built-in tool invocations.
    rate_limiter: RateLimiter,
    /// Optional HTTP interceptor propagated into registered WASM wrappers.
    http_interceptor: Option<Arc<dyn HttpInterceptor>>,
    /// Reference to the message tool for setting context per-turn.
    message_tool: RwLock<Option<Arc<crate::tools::builtin::MessageTool>>>,
    /// Active engine version. Controls which tools are visible via
    /// `tool_definitions()`, `all()`, etc. Defaults to V1.
    engine_version: EngineVersion,
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

    fn is_engine_visible(tool: &dyn Tool, version: EngineVersion) -> bool {
        tool.engine_compatibility().is_visible_in(version)
    }

    /// Create a new empty registry. Defaults to engine V1.
    pub fn new() -> Self {
        Self {
            tools: RwLock::new(HashMap::new()),
            builtin_names: RwLock::new(std::collections::HashSet::new()),
            credential_registry: None,
            secrets_store: None,
            role_lookup: None,
            db: None,
            rate_limiter: RateLimiter::new(),
            http_interceptor: None,
            message_tool: RwLock::new(None),
            engine_version: EngineVersion::V1,
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

    /// Attach a database handle for user-role aware tool behavior.
    pub fn with_database(mut self, db: Arc<dyn Database>) -> Self {
        let role_lookup: Arc<dyn UserStore> = db.clone();
        self.role_lookup = Some(role_lookup);
        self.db = Some(db);
        self
    }

    pub fn with_role_lookup(mut self, role_lookup: Arc<dyn UserStore>) -> Self {
        self.role_lookup = Some(role_lookup);
        self
    }

    pub fn with_http_interceptor(mut self, interceptor: Arc<dyn HttpInterceptor>) -> Self {
        self.http_interceptor = Some(interceptor);
        self
    }

    /// Set the engine version. Must be called before wrapping in `Arc`.
    pub fn with_engine_version(mut self, version: EngineVersion) -> Self {
        self.engine_version = version;
        self
    }

    /// Get the active engine version.
    pub fn engine_version(&self) -> EngineVersion {
        self.engine_version
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

    pub fn database(&self) -> Option<&Arc<dyn Database>> {
        self.db.as_ref()
    }

    pub fn role_lookup(&self) -> Option<&Arc<dyn UserStore>> {
        self.role_lookup.as_ref()
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

    /// Resolve a tool name to the key under which it is registered,
    /// trying the exact name first, then hyphen→underscore and
    /// underscore→hyphen aliases.
    fn resolve_key(tools: &HashMap<String, Arc<dyn Tool>>, name: &str) -> Option<String> {
        if tools.contains_key(name) {
            return Some(name.to_string());
        }
        // Reverse alias: hyphens → underscores (LLM normalization)
        let underscore_alias = name.replace('-', "_");
        if underscore_alias != name && tools.contains_key(&underscore_alias) {
            return Some(underscore_alias);
        }
        // Legacy alias: underscores → hyphens (older WASM extensions)
        let hyphen_alias = name.replace('_', "-");
        if hyphen_alias != name && tools.contains_key(&hyphen_alias) {
            return Some(hyphen_alias);
        }
        None
    }

    /// Unregister a tool.  Uses the same alias resolution as `get()` so
    /// callers that pass hyphenated names still find underscore-registered
    /// tools.
    pub async fn unregister(&self, name: &str) -> Option<Arc<dyn Tool>> {
        let mut tools = self.tools.write().await;
        let key = Self::resolve_key(&tools, name)?;
        tools.remove(&key)
    }

    /// Get a tool by name.
    ///
    /// Falls back to a hyphen→underscore alias when the exact name is not
    /// found, so that tool calls from LLM providers that normalise hyphens
    /// (e.g. `notion_notion_search` vs the registered `notion_notion-search`)
    /// still resolve correctly.
    pub async fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        let tools = self.tools.read().await;
        let key = Self::resolve_key(&tools, name)?;
        tools.get(&key).map(Arc::clone)
    }

    /// Resolve a caller-provided action/tool name to the registered tool id.
    ///
    /// Tries exact match first, then hyphen→underscore (LLM normalization),
    /// then underscore→hyphen (legacy WASM extensions).
    pub async fn resolve_name(&self, name: &str) -> Option<String> {
        let tools = self.tools.read().await;
        Self::resolve_key(&tools, name)
    }

    pub async fn get_resolved(&self, name: &str) -> Option<(String, Arc<dyn Tool>)> {
        let tools = self.tools.read().await;
        let key = Self::resolve_key(&tools, name)?;
        let tool = tools.get(&key).map(Arc::clone)?;
        Some((key, tool))
    }

    /// Resolve a tool/action name to its owning provider extension, when the
    /// action is extension-backed.
    pub async fn provider_extension_for_tool(&self, name: &str) -> Option<String> {
        let tools = self.tools.read().await;
        let key = Self::resolve_key(&tools, name)?;
        tools
            .get(&key)
            .and_then(|tool| tool.provider_extension().map(ToOwned::to_owned))
    }

    /// Check if a tool exists.
    pub async fn has(&self, name: &str) -> bool {
        self.get(name).await.is_some()
    }

    /// List tool names visible in the current engine version.
    pub async fn list(&self) -> Vec<String> {
        let version = self.engine_version;
        self.tools
            .read()
            .await
            .values()
            .filter(|tool| Self::is_engine_visible(tool.as_ref(), version))
            .map(|tool| tool.name().to_string())
            .collect()
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

    /// Get all tools visible in the current engine version.
    pub async fn all(&self) -> Vec<Arc<dyn Tool>> {
        let version = self.engine_version;
        self.tools
            .read()
            .await
            .values()
            .filter(|tool| Self::is_engine_visible(tool.as_ref(), version))
            .cloned()
            .collect()
    }

    /// Get the set of built-in tool names currently registered.
    pub async fn builtin_tool_names(&self) -> std::collections::HashSet<String> {
        self.builtin_names.read().await.clone()
    }

    /// Get tool definitions for LLM function calling.
    ///
    /// Automatically filters by the registry's engine version, so callers
    /// don't need to know which engine is active.
    pub async fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tool_definitions_for_engine(self.engine_version).await
    }

    /// Get tool definitions filtered by engine version.
    ///
    /// Returns tools whose `engine_compatibility()` is `Both` or matches the
    /// requested version. Use this instead of `tool_definitions()` when building
    /// the tool list for a specific engine version.
    pub async fn tool_definitions_for_engine(&self, version: EngineVersion) -> Vec<ToolDefinition> {
        let mut defs: Vec<ToolDefinition> = self
            .tools
            .read()
            .await
            .values()
            .filter(|tool| Self::is_engine_visible(tool.as_ref(), version))
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
            .filter_map(|name| {
                let key = Self::resolve_key(&tools, name)?;
                tools.get(&key).map(Self::tool_definition)
            })
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
        if let Some(role_lookup) = &self.role_lookup {
            http = http.with_role_lookup(Arc::clone(role_lookup));
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

    /// Register system introspection tools (tools_list, version).
    ///
    /// Requires `Arc<Self>` because `SystemToolsListTool` queries the
    /// registry at runtime. Call after other registration methods.
    pub fn register_system_tools(self: &Arc<Self>) {
        use crate::tools::builtin::system::{SystemToolsListTool, SystemVersionTool};
        self.register_sync(Arc::new(SystemToolsListTool::new(Arc::clone(self))));
        self.register_sync(Arc::new(SystemVersionTool));
        tracing::debug!("Registered system introspection tools");
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
        let version = self.engine_version;
        self.tools
            .read()
            .await
            .values()
            .filter(|tool| {
                tool.domain() == domain && Self::is_engine_visible(tool.as_ref(), version)
            })
            .map(Self::tool_definition)
            .collect()
    }

    /// Get tool definitions excluding specific tools by name.
    ///
    /// Used by lightweight routines to filter out denylisted and approval-gated tools
    /// so the LLM only sees tools it is actually allowed to call.
    pub async fn tool_definitions_excluding(&self, deny: &[&str]) -> Vec<ToolDefinition> {
        let empty_params = serde_json::Value::Object(serde_json::Map::new());
        let version = self.engine_version;
        let mut defs: Vec<ToolDefinition> = self
            .tools
            .read()
            .await
            .values()
            .filter(|tool| {
                if !Self::is_engine_visible(tool.as_ref(), version) {
                    return false;
                }
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
        let file_history = shared_file_history();
        let read_state = shared_read_file_state();

        self.register_sync(Arc::new(ShellTool::new()));
        self.register_sync(Arc::new(
            ReadFileTool::new().with_read_state(Arc::clone(&read_state)),
        ));
        self.register_sync(Arc::new(
            WriteFileTool::new()
                .with_file_history(Arc::clone(&file_history))
                .with_read_state(Arc::clone(&read_state)),
        ));
        self.register_sync(Arc::new(ListDirTool::new()));
        self.register_sync(Arc::new(
            ApplyPatchTool::new()
                .with_file_history(Arc::clone(&file_history))
                .with_read_state(Arc::clone(&read_state)),
        ));
        self.register_sync(Arc::new(GlobTool::new()));
        self.register_sync(Arc::new(GrepTool::new()));
        self.register_sync(Arc::new(FileUndoTool::new(file_history)));

        tracing::debug!("Registered 8 development tools");
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
    /// When `catalog` is `None` (ClawHub disabled), `skill_search` is omitted
    /// and `skill_install` only accepts direct URL or content.
    pub fn register_skill_tools(
        &self,
        registry: Arc<std::sync::RwLock<SkillRegistry>>,
        catalog: Option<Arc<SkillCatalog>>,
    ) {
        self.register_sync(Arc::new(SkillListTool::new(Arc::clone(&registry))));
        self.register_sync(Arc::new(SkillRemoveTool::new(Arc::clone(&registry))));

        if let Some(ref catalog) = catalog {
            self.register_sync(Arc::new(SkillSearchTool::new(
                Arc::clone(&registry),
                Arc::clone(catalog),
            )));
        }

        self.register_sync(Arc::new(SkillInstallTool::new(registry, catalog.clone())));

        tracing::debug!(
            "Registered skill management tools (clawhub={})",
            catalog.is_some()
        );
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
        let oauth_refresh = reg.oauth_refresh.clone();

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
        if let Some(role_lookup) = reg.role_lookup {
            wrapper = wrapper.with_role_lookup(role_lookup);
        }
        if let Some(oauth) = oauth_refresh.clone() {
            wrapper = wrapper.with_oauth_refresh(oauth);
        }
        if let Some(interceptor) = &self.http_interceptor {
            wrapper = wrapper.with_http_interceptor(Arc::clone(interceptor));
        }

        // Register the tool
        self.register(Arc::new(wrapper)).await;

        // Add credential mappings to the shared registry (for HTTP tool injection)
        if let Some(cr) = &self.credential_registry
            && !credential_mappings.is_empty()
        {
            let count = credential_mappings.len();
            cr.add_mappings(credential_mappings);
            if let Some(oauth) = oauth_refresh {
                cr.add_oauth_refresh_configs(std::iter::once((oauth.secret_name.clone(), oauth)));
            }
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
            role_lookup: self.role_lookup.clone(),
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
    /// Narrow role lookup for user-role aware fallback decisions.
    pub role_lookup: Option<Arc<dyn UserStore>>,
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
    use crate::tools::tool::{EngineCompatibility, ToolDiscoverySummary};

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
    async fn provider_extension_lookup_uses_tool_metadata() {
        struct ProviderTool;

        #[async_trait::async_trait]
        impl Tool for ProviderTool {
            fn name(&self) -> &str {
                "notion_search"
            }

            fn description(&self) -> &str {
                "provider tool"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }

            fn provider_extension(&self) -> Option<&str> {
                Some("notion")
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
        registry.register(Arc::new(ProviderTool)).await;

        assert_eq!(
            registry.provider_extension_for_tool("notion_search").await,
            Some("notion".to_string())
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

    // ── engine compatibility tests ───────────────────────────────────────

    /// Stub tool that returns V1Only engine compatibility.
    struct V1OnlyTool;

    #[async_trait::async_trait]
    impl crate::tools::Tool for V1OnlyTool {
        fn name(&self) -> &str {
            "v1_only_stub"
        }
        fn description(&self) -> &str {
            "test stub"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &crate::context::JobContext,
        ) -> Result<crate::tools::ToolOutput, crate::tools::ToolError> {
            unreachable!()
        }
        fn engine_compatibility(&self) -> EngineCompatibility {
            EngineCompatibility::V1Only
        }
    }

    #[tokio::test]
    async fn tool_definitions_for_engine_excludes_v1_only_from_v2() {
        let registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool)).await;
        registry.register(Arc::new(V1OnlyTool)).await;

        let v2_defs = registry
            .tool_definitions_for_engine(EngineVersion::V2)
            .await;
        let names: Vec<&str> = v2_defs.iter().map(|d| d.name.as_str()).collect();

        assert!(
            names.contains(&"echo"),
            "Both-compatible tool should appear in v2"
        );
        assert!(
            !names.contains(&"v1_only_stub"),
            "V1Only tool must not appear in v2"
        );
    }

    #[tokio::test]
    async fn tool_definitions_for_engine_includes_v1_only_in_v1() {
        let registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool)).await;
        registry.register(Arc::new(V1OnlyTool)).await;

        let v1_defs = registry
            .tool_definitions_for_engine(EngineVersion::V1)
            .await;
        let names: Vec<&str> = v1_defs.iter().map(|d| d.name.as_str()).collect();

        assert!(
            names.contains(&"echo"),
            "Both-compatible tool should appear in v1"
        );
        assert!(
            names.contains(&"v1_only_stub"),
            "V1Only tool should appear in v1"
        );
    }

    #[tokio::test]
    async fn builtin_echo_tool_is_both_compatible() {
        let registry = Arc::new(ToolRegistry::new());
        registry.register_builtin_tools();

        let echo = registry.get("echo").await.unwrap();
        assert_eq!(echo.engine_compatibility(), EngineCompatibility::Both);
    }

    #[tokio::test]
    async fn tool_definitions_auto_filters_by_stored_engine_version() {
        let registry = ToolRegistry::new().with_engine_version(EngineVersion::V2);
        registry.register(Arc::new(EchoTool)).await;
        registry.register(Arc::new(V1OnlyTool)).await;

        // tool_definitions() should auto-filter using the stored V2 version
        let defs = registry.tool_definitions().await;
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();

        assert!(names.contains(&"echo"));
        assert!(!names.contains(&"v1_only_stub"));
    }

    #[tokio::test]
    async fn default_v1_registry_includes_v1_only_tools() {
        // Default ToolRegistry::new() is V1 — V1Only tools should be visible
        let registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool)).await;
        registry.register(Arc::new(V1OnlyTool)).await;

        let defs = registry.tool_definitions().await;
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();

        assert!(names.contains(&"echo"));
        assert!(names.contains(&"v1_only_stub"));
    }

    #[tokio::test]
    async fn all_filters_by_engine_version() {
        let registry = ToolRegistry::new().with_engine_version(EngineVersion::V2);
        registry.register(Arc::new(EchoTool)).await;
        registry.register(Arc::new(V1OnlyTool)).await;

        let tools = registry.all().await;
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();

        assert!(names.contains(&"echo"));
        assert!(!names.contains(&"v1_only_stub"));
    }

    /// Regression test: tool names with hyphens must be resolvable when the
    /// LLM provider normalises hyphens to underscores (nearai/ironclaw#NNN).
    #[tokio::test]
    async fn get_resolves_hyphen_to_underscore_alias() {
        let registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool)).await;

        // Register a tool whose name contains underscores (the normalised
        // form produced by the MCP prefixed-name fix).
        struct UnderscoreTool;
        #[async_trait::async_trait]
        impl Tool for UnderscoreTool {
            fn name(&self) -> &str {
                "notion_notion_search"
            }
            fn description(&self) -> &str {
                "test"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({"type": "object"})
            }
            async fn execute(
                &self,
                _params: serde_json::Value,
                _ctx: &crate::context::JobContext,
            ) -> Result<crate::tools::tool::ToolOutput, crate::tools::tool::ToolError> {
                unreachable!()
            }
        }
        registry.register(Arc::new(UnderscoreTool)).await;

        // Exact match works
        assert!(registry.get("notion_notion_search").await.is_some());
        // Hyphenated variant resolves via alias
        assert!(registry.get("notion_notion-search").await.is_some());
        // has() also resolves
        assert!(registry.has("notion_notion-search").await);
        // Completely wrong name still fails
        assert!(registry.get("nonexistent_tool").await.is_none());
    }

    /// Regression test: legacy underscore→hyphen alias still works.
    #[tokio::test]
    async fn get_resolves_underscore_to_hyphen_alias() {
        let registry = ToolRegistry::new();

        struct HyphenTool;
        #[async_trait::async_trait]
        impl Tool for HyphenTool {
            fn name(&self) -> &str {
                "my-old-tool"
            }
            fn description(&self) -> &str {
                "test"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({"type": "object"})
            }
            async fn execute(
                &self,
                _params: serde_json::Value,
                _ctx: &crate::context::JobContext,
            ) -> Result<crate::tools::tool::ToolOutput, crate::tools::tool::ToolError> {
                unreachable!()
            }
        }
        registry.register(Arc::new(HyphenTool)).await;

        // Exact hyphenated match works
        assert!(registry.get("my-old-tool").await.is_some());
        // Underscored variant resolves via legacy alias
        assert!(registry.get("my_old_tool").await.is_some());
    }

    /// Regression test: get_resolved must use resolve_key so that dispatch,
    /// approval gate, and effect adapter all resolve hyphenated names.
    #[tokio::test]
    async fn get_resolved_hyphen_to_underscore() {
        let registry = ToolRegistry::new();

        struct UnderscoreTool;
        #[async_trait::async_trait]
        impl Tool for UnderscoreTool {
            fn name(&self) -> &str {
                "notion_notion_search"
            }
            fn description(&self) -> &str {
                "test"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({"type": "object"})
            }
            async fn execute(
                &self,
                _params: serde_json::Value,
                _ctx: &crate::context::JobContext,
            ) -> Result<crate::tools::tool::ToolOutput, crate::tools::tool::ToolError> {
                unreachable!()
            }
        }
        registry.register(Arc::new(UnderscoreTool)).await;

        let (key, _) = registry
            .get_resolved("notion_notion-search")
            .await
            .expect("get_resolved must resolve hyphenated name to underscore registration");
        assert_eq!(key, "notion_notion_search");
    }

    /// Regression test: unregister with alias resolution.
    #[tokio::test]
    async fn unregister_resolves_hyphen_alias() {
        let registry = ToolRegistry::new();

        struct TestTool;
        #[async_trait::async_trait]
        impl Tool for TestTool {
            fn name(&self) -> &str {
                "my_mcp_search"
            }
            fn description(&self) -> &str {
                "test"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({"type": "object"})
            }
            async fn execute(
                &self,
                _params: serde_json::Value,
                _ctx: &crate::context::JobContext,
            ) -> Result<crate::tools::tool::ToolOutput, crate::tools::tool::ToolError> {
                unreachable!()
            }
        }
        registry.register(Arc::new(TestTool)).await;
        assert!(registry.has("my_mcp_search").await);

        // Unregister using hyphenated alias
        let removed = registry.unregister("my-mcp-search").await;
        assert!(removed.is_some(), "unregister must resolve hyphen alias");
        assert!(!registry.has("my_mcp_search").await, "tool should be gone");
    }
    #[tokio::test]
    async fn test_register_skill_tools_without_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let registry = ToolRegistry::new();
        let sr = Arc::new(std::sync::RwLock::new(crate::skills::SkillRegistry::new(
            dir.path().to_path_buf(),
        )));
        registry.register_skill_tools(sr, None);
        assert!(registry.has("skill_list").await);
        assert!(registry.has("skill_remove").await);
        assert!(registry.has("skill_install").await);
        assert!(
            !registry.has("skill_search").await,
            "skill_search should not be registered without catalog"
        );
    }
}
