//! Tool registry for managing available tools.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::context::ContextManager;
use crate::db::Database;
use crate::extensions::ExtensionManager;
use crate::llm::{LlmProvider, ToolDefinition};
use crate::orchestrator::job_manager::ContainerJobManager;
use crate::sandbox::proxy::DomainAllowlist;
use crate::secrets::SecretsStore;
use crate::skills::catalog::SkillCatalog;
use crate::skills::registry::SkillRegistry;
use crate::tools::builder::{
    BuildSoftwareTool, BuilderConfig, LlmSoftwareBuilder, SoftwareBuilder,
};
use crate::tools::builtin::{
    ApplyPatchTool, CancelJobTool, CreateJobTool, EchoTool, ExtensionInfoTool, HttpTool,
    JobEventsTool, JobPromptTool, JobStatusTool, JsonTool, ListDirTool, ListJobsTool,
    MemoryReadTool, MemorySearchTool, MemoryTreeTool, MemoryWriteTool, PromptQueue, ReadFileTool,
    ShellTool, SkillInstallTool, SkillListTool, SkillRemoveTool, SkillSearchTool, TimeTool,
    ToolActivateTool, ToolAuthTool, ToolInstallTool, ToolListTool, ToolRemoveTool, ToolSearchTool,
    ToolUpgradeTool, WriteFileTool,
};
use crate::tools::rate_limiter::RateLimiter;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolDomain};
use crate::tools::wasm::{
    Capabilities, OAuthRefreshConfig, ResourceLimits, SharedCredentialRegistry, WasmError,
    WasmStorageError, WasmToolRuntime, WasmToolStore, WasmToolWrapper,
};
use crate::workspace::Workspace;

/// Names of built-in tools that cannot be shadowed by dynamic registrations.
/// This prevents a dynamically built or installed tool from replacing a
/// security-critical built-in like "shell" or "memory_write".
const PROTECTED_TOOL_NAMES: &[&str] = &[
    "echo",
    "time",
    "json",
    "http",
    "shell",
    "read_file",
    "write_file",
    "list_dir",
    "apply_patch",
    "memory_search",
    "memory_write",
    "memory_read",
    "memory_tree",
    "create_job",
    "list_jobs",
    "job_status",
    "cancel_job",
    "build_software",
    "tool_search",
    "tool_install",
    "tool_auth",
    "tool_activate",
    "tool_list",
    "tool_remove",
    "routine_create",
    "routine_list",
    "routine_update",
    "routine_delete",
    "routine_fire",
    "routine_history",
    "event_emit",
    "skill_list",
    "skill_search",
    "skill_install",
    "skill_remove",
    "message",
    "web_fetch",
    "restart",
    "image_generate",
    "image_edit",
    "image_analyze",
    "tool_info",
];

struct HttpToolWithApprovalBypass {
    inner: HttpTool,
    allowlist: DomainAllowlist,
}

impl HttpToolWithApprovalBypass {
    fn new(inner: HttpTool, allowed_domains: &[String]) -> Self {
        Self {
            inner,
            allowlist: DomainAllowlist::new(allowed_domains),
        }
    }

    fn host_from_params(params: &serde_json::Value) -> Option<String> {
        params
            .get("url")
            .and_then(|u| u.as_str())
            .and_then(|u| reqwest::Url::parse(u).ok())
            .and_then(|u| u.host_str().map(|h| h.to_string()))
    }

    fn request_is_whitelisted(&self, params: &serde_json::Value) -> bool {
        Self::host_from_params(params)
            .is_some_and(|host| self.allowlist.is_allowed(&host).is_allowed())
    }
}

#[async_trait::async_trait]
impl Tool for HttpToolWithApprovalBypass {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.inner.parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &crate::context::JobContext,
    ) -> Result<crate::tools::tool::ToolOutput, crate::tools::tool::ToolError> {
        self.inner.execute(params, ctx).await
    }

    fn estimated_cost(&self, params: &serde_json::Value) -> Option<rust_decimal::Decimal> {
        self.inner.estimated_cost(params)
    }

    fn estimated_duration(&self, params: &serde_json::Value) -> Option<std::time::Duration> {
        self.inner.estimated_duration(params)
    }

    fn requires_sanitization(&self) -> bool {
        self.inner.requires_sanitization()
    }

    fn requires_approval(&self, params: &serde_json::Value) -> ApprovalRequirement {
        if self.request_is_whitelisted(params) {
            return ApprovalRequirement::Never;
        }

        self.inner.requires_approval(params)
    }

    fn execution_timeout(&self) -> std::time::Duration {
        self.inner.execution_timeout()
    }

    fn domain(&self) -> ToolDomain {
        self.inner.domain()
    }

    fn sensitive_params(&self) -> &[&str] {
        self.inner.sensitive_params()
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        self.inner.rate_limit_config()
    }

    fn webhook_capability(&self) -> Option<crate::tools::wasm::WebhookCapability> {
        self.inner.webhook_capability()
    }

    fn discovery_schema(&self) -> serde_json::Value {
        self.inner.discovery_schema()
    }

    fn discovery_summary(&self) -> Option<crate::tools::tool::ToolDiscoverySummary> {
        self.inner.discovery_summary()
    }
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
    /// HTTP domains that bypass approval prompts for the built-in HTTP tool.
    http_allowed_domains: Vec<String>,
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
            http_allowed_domains: Vec::new(),
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

    /// Configure domains that bypass approval for the built-in HTTP tool.
    pub fn with_http_allowed_domains(mut self, domains: Vec<String>) -> Self {
        self.http_allowed_domains = domains;
        self
    }

    /// Get a reference to the shared credential registry.
    pub fn credential_registry(&self) -> Option<&Arc<SharedCredentialRegistry>> {
        self.credential_registry.as_ref()
    }

    /// Get the shared rate limiter for checking built-in tool limits.
    pub fn rate_limiter(&self) -> &RateLimiter {
        &self.rate_limiter
    }

    /// Register a tool. Rejects dynamic tools that try to shadow a protected built-in name.
    pub async fn register(&self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
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
    pub fn register_sync(&self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
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

        let mut http = HttpTool::new();
        if let (Some(cr), Some(ss)) = (&self.credential_registry, &self.secrets_store) {
            http = http.with_credentials(Arc::clone(cr), Arc::clone(ss));
        }
        self.register_sync(Arc::new(HttpToolWithApprovalBypass::new(
            http,
            &self.http_allowed_domains,
        )));

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

    /// Register memory tools with a workspace.
    ///
    /// Memory tools require a workspace for persistence. Call this after
    /// `register_builtin_tools()` if you have a workspace available.
    pub fn register_memory_tools(&self, workspace: Arc<Workspace>) {
        self.register_sync(Arc::new(MemorySearchTool::new(Arc::clone(&workspace))));
        self.register_sync(Arc::new(MemoryWriteTool::new(Arc::clone(&workspace))));
        self.register_sync(Arc::new(MemoryReadTool::new(Arc::clone(&workspace))));
        self.register_sync(Arc::new(MemoryTreeTool::new(workspace)));

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
            tokio::sync::broadcast::Sender<(uuid::Uuid, crate::channels::web::types::SseEvent)>,
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

        tracing::info!("Registered software builder tool");
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
    async fn test_tool_definitions_use_tool_schema() -> Result<(), Box<dyn std::error::Error>> {
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
            .ok_or_else(|| std::io::Error::other("tool definition should be present"))?;
        if !def.description.contains("tool_info") {
            return Err(std::io::Error::other(format!(
                "live tool definition should include schema hint: {}",
                def.description
            ))
            .into());
        }
        if def.parameters.get("extra").is_some() {
            return Err(std::io::Error::other("tool_info schema should not expose extra").into());
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_builtin_tool_cannot_be_shadowed() -> Result<(), Box<dyn std::error::Error>> {
        let registry = ToolRegistry::new();
        // Register echo as built-in (uses register_sync and echo is protected).
        registry.register_sync(Arc::new(EchoTool));
        if !registry.has("echo").await {
            return Err(std::io::Error::other("echo should be present").into());
        }

        let original_tool = registry
            .get("echo")
            .await
            .ok_or_else(|| std::io::Error::other("echo should be present"))?;
        let original_desc = original_tool.description().to_string();

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
        let current_tool = registry
            .get("echo")
            .await
            .ok_or_else(|| std::io::Error::other("echo should be present"))?;
        let desc = current_tool.description().to_string();
        if desc != original_desc {
            return Err(std::io::Error::other("echo description changed unexpectedly").into());
        }
        if desc == "EVIL SHADOW" {
            return Err(std::io::Error::other("shadow tool replaced built-in echo").into());
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_builtin_tool_names_include_non_protected_sync_tools()
    -> Result<(), Box<dyn std::error::Error>> {
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
        if !builtins.contains("owner_gate") {
            return Err(std::io::Error::other("owner_gate should be tracked as builtin").into());
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_register_and_read_no_panic() -> Result<(), Box<dyn std::error::Error>> {
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
                if tools.is_empty() {
                    return Err(std::io::Error::other("tools should not be empty"));
                }
                let names = reg.list().await;
                if names.is_empty() {
                    return Err(std::io::Error::other("names should not be empty"));
                }
                let _ = reg.get("echo").await;
                let _ = reg.has("echo").await;
                let _ = reg.tool_definitions().await;
                Ok::<(), std::io::Error>(())
            }));
        }

        // Concurrent register attempts (will be rejected as shadowing)
        for _ in 0..5 {
            let reg = StdArc::clone(&registry);
            handles.push(tokio::spawn(async move {
                // This will be rejected (echo is protected) but should not panic
                reg.register(Arc::new(EchoTool)).await;
                Ok::<(), std::io::Error>(())
            }));
        }

        for handle in handles {
            handle.await??;
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_tool_definitions_sorted_alphabetically() -> Result<(), Box<dyn std::error::Error>>
    {
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
        registry.register(Arc::new(ToolZ)).await;
        registry.register(Arc::new(ToolA)).await;
        registry.register(Arc::new(ToolM)).await;

        let defs = registry.tool_definitions().await;
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        if names != vec!["alpha", "middle", "zebra"] {
            return Err(
                std::io::Error::other("tool definitions should be sorted alphabetically").into(),
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_retain_only_filters_tools() -> Result<(), Box<dyn std::error::Error>> {
        let registry = ToolRegistry::new();
        registry.register_builtin_tools();
        let all = registry.list().await;
        if all.len() <= 2 {
            return Err(std::io::Error::other("expected multiple built-in tools").into());
        }
        registry.retain_only(&["echo", "time"]).await;
        let remaining = registry.list().await;
        if remaining.len() != 2 {
            return Err(std::io::Error::other("expected two remaining tools").into());
        }
        if !remaining.contains(&"echo".to_string()) || !remaining.contains(&"time".to_string()) {
            return Err(std::io::Error::other("expected echo and time to remain").into());
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_retain_only_empty_is_noop() -> Result<(), Box<dyn std::error::Error>> {
        let registry = ToolRegistry::new();
        registry.register_builtin_tools();
        let before = registry.list().await.len();
        registry.retain_only(&[]).await;
        let after = registry.list().await.len();
        if before != after {
            return Err(std::io::Error::other("retain_only([]) should be a no-op").into());
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_http_allowlisted_domain_bypasses_approval()
    -> Result<(), Box<dyn std::error::Error>> {
        let registry = ToolRegistry::new().with_http_allowed_domains(vec![
            "api.example.com".to_string(),
            "*.vercel.app".to_string(),
        ]);
        registry.register_builtin_tools();

        let http_tool = registry
            .get("http")
            .await
            .ok_or_else(|| std::io::Error::other("http tool should be present"))?;

        if http_tool.requires_approval(&serde_json::json!({
            "method": "POST",
            "url": "https://api.example.com/v1/messages"
        })) != ApprovalRequirement::Never
        {
            return Err(
                std::io::Error::other("exact allowlisted domain should bypass approval").into(),
            );
        }
        if http_tool.requires_approval(&serde_json::json!({
            "method": "GET",
            "url": "https://preview.vercel.app/api"
        })) != ApprovalRequirement::Never
        {
            return Err(std::io::Error::other(
                "wildcard allowlisted domain should bypass approval",
            )
            .into());
        }
        if http_tool.requires_approval(&serde_json::json!({
            "method": "POST",
            "url": "https://evil.example.net"
        })) != ApprovalRequirement::UnlessAutoApproved
        {
            return Err(
                std::io::Error::other("unlisted domain should still require approval").into(),
            );
        }
        Ok(())
    }
}
