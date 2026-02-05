//! Software builder for creating programs and tools using LLM-driven code generation.
//!
//! This module provides a general-purpose software building capability that:
//! - Uses an agent loop similar to Codex for iterative development
//! - Can build any software (binaries, libraries, scripts)
//! - Has special context injection when building WASM tools
//! - Integrates with existing tool loading infrastructure
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────┐
//! │                          Software Build Loop                                 │
//! │                                                                              │
//! │  1. Analyze requirement ─▶ Determine project type, language, structure      │
//! │  2. Generate scaffold   ─▶ Create initial project files                     │
//! │  3. Implement code      ─▶ Write the actual implementation                  │
//! │  4. Build/compile       ─▶ Run build commands (cargo, npm, etc.)            │
//! │  5. Fix errors          ─▶ Parse errors, modify code, retry                 │
//! │  6. Test                ─▶ Run tests, fix failures                          │
//! │  7. Package             ─▶ Produce final artifact                           │
//! └─────────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! For WASM tools specifically:
//! - Injects Tool trait interface documentation
//! - Injects WASM host function documentation
//! - Compiles to wasm32-wasip2 target
//! - Validates against tool interface
//! - Registers with ToolRegistry

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::JobContext;
use crate::error::ToolError as AgentToolError;
use crate::llm::{
    ChatMessage, LlmProvider, Reasoning, ReasoningContext, RespondResult, ToolDefinition,
};
use crate::safety::SafetyLayer;
use crate::tools::ToolRegistry;
use crate::tools::tool::{Tool, ToolError, ToolOutput};

/// Requirement specification for building software.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildRequirement {
    /// Name for the software.
    pub name: String,
    /// Description of what it should do.
    pub description: String,
    /// Type of software to build.
    pub software_type: SoftwareType,
    /// Target language/runtime.
    pub language: Language,
    /// Expected input format (for tools/CLIs).
    pub input_spec: Option<String>,
    /// Expected output format.
    pub output_spec: Option<String>,
    /// External dependencies needed.
    pub dependencies: Vec<String>,
    /// Security/capability requirements (for WASM tools).
    pub capabilities: Vec<String>,
}

/// Type of software being built.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SoftwareType {
    /// A WASM tool for the agent.
    WasmTool,
    /// A standalone CLI application.
    CliBinary,
    /// A library/crate.
    Library,
    /// A script (Python, Bash, etc.).
    Script,
    /// A web service/API.
    WebService,
}

/// Programming language for the build.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Rust,
    Python,
    TypeScript,
    JavaScript,
    Go,
    Bash,
}

impl Language {
    /// Get the file extension for this language.
    pub fn extension(&self) -> &'static str {
        match self {
            Language::Rust => "rs",
            Language::Python => "py",
            Language::TypeScript => "ts",
            Language::JavaScript => "js",
            Language::Go => "go",
            Language::Bash => "sh",
        }
    }

    /// Get the build command for this language.
    pub fn build_command(&self, project_dir: &str) -> Option<String> {
        match self {
            Language::Rust => Some(format!("cd {} && cargo build --release", project_dir)),
            Language::TypeScript => Some(format!("cd {} && npm run build", project_dir)),
            Language::Go => Some(format!("cd {} && go build ./...", project_dir)),
            Language::Python | Language::JavaScript | Language::Bash => None, // Interpreted
        }
    }

    /// Get the test command for this language.
    pub fn test_command(&self, project_dir: &str) -> String {
        match self {
            Language::Rust => format!("cd {} && cargo test", project_dir),
            Language::Python => format!("cd {} && python -m pytest", project_dir),
            Language::TypeScript | Language::JavaScript => {
                format!("cd {} && npm test", project_dir)
            }
            Language::Go => format!("cd {} && go test ./...", project_dir),
            Language::Bash => format!("cd {} && shellcheck *.sh", project_dir),
        }
    }
}

/// Result of a build operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildResult {
    /// Unique ID for this build.
    pub build_id: Uuid,
    /// The requirement that was built.
    pub requirement: BuildRequirement,
    /// Path to the output artifact.
    pub artifact_path: PathBuf,
    /// Build logs.
    pub logs: Vec<BuildLog>,
    /// Whether the build succeeded.
    pub success: bool,
    /// Error message if failed.
    pub error: Option<String>,
    /// When the build started.
    pub started_at: DateTime<Utc>,
    /// When the build completed.
    pub completed_at: DateTime<Utc>,
    /// Number of iterations to complete.
    pub iterations: u32,
    /// Validation warnings (for WASM tools).
    #[serde(default)]
    pub validation_warnings: Vec<String>,
    /// Test results summary.
    #[serde(default)]
    pub tests_passed: u32,
    /// Number of tests that failed.
    #[serde(default)]
    pub tests_failed: u32,
    /// Whether the tool was auto-registered (for WASM tools).
    #[serde(default)]
    pub registered: bool,
}

/// A log entry from the build process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildLog {
    pub timestamp: DateTime<Utc>,
    pub phase: BuildPhase,
    pub message: String,
    pub details: Option<String>,
}

/// Phases of the build process.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BuildPhase {
    Analyzing,
    Scaffolding,
    Implementing,
    Building,
    Testing,
    Fixing,
    Validating,
    Registering,
    Packaging,
    Complete,
    Failed,
}

/// Configuration for the software builder.
#[derive(Debug, Clone)]
pub struct BuilderConfig {
    /// Directory where builds happen.
    pub build_dir: PathBuf,
    /// Maximum iterations before giving up.
    pub max_iterations: u32,
    /// Timeout for the entire build.
    pub timeout: Duration,
    /// Whether to clean up failed builds.
    pub cleanup_on_failure: bool,
    /// Whether to validate WASM tools after building.
    pub validate_wasm: bool,
    /// Whether to run tests after building.
    pub run_tests: bool,
    /// Whether to auto-register successful WASM tool builds.
    pub auto_register: bool,
    /// Directory to copy successful WASM tools for persistence.
    pub wasm_output_dir: Option<PathBuf>,
}

impl Default for BuilderConfig {
    fn default() -> Self {
        Self {
            build_dir: std::env::temp_dir().join("ironclaw-builds"),
            max_iterations: 10,
            timeout: Duration::from_secs(600), // 10 minutes
            cleanup_on_failure: false,         // Keep for debugging
            validate_wasm: true,
            run_tests: true,
            auto_register: true,
            wasm_output_dir: None,
        }
    }
}

/// Trait for building software.
#[async_trait]
pub trait SoftwareBuilder: Send + Sync {
    /// Analyze a natural language description and extract a structured requirement.
    async fn analyze(&self, description: &str) -> Result<BuildRequirement, AgentToolError>;

    /// Build software from a requirement.
    async fn build(&self, requirement: &BuildRequirement) -> Result<BuildResult, AgentToolError>;

    /// Attempt to repair a failed build.
    async fn repair(
        &self,
        result: &BuildResult,
        error: &str,
    ) -> Result<BuildResult, AgentToolError>;
}

/// LLM-powered software builder.
pub struct LlmSoftwareBuilder {
    config: BuilderConfig,
    llm: Arc<dyn LlmProvider>,
    safety: Arc<SafetyLayer>,
    tools: Arc<ToolRegistry>,
}

impl LlmSoftwareBuilder {
    /// Create a new LLM-based software builder.
    pub fn new(
        config: BuilderConfig,
        llm: Arc<dyn LlmProvider>,
        safety: Arc<SafetyLayer>,
        tools: Arc<ToolRegistry>,
    ) -> Self {
        // Ensure build directory exists
        if let Err(e) = std::fs::create_dir_all(&config.build_dir) {
            tracing::warn!("Failed to create build directory: {}", e);
        }

        Self {
            config,
            llm,
            safety,
            tools,
        }
    }

    /// Get the build tools available for the build loop.
    async fn get_build_tools(&self) -> Vec<ToolDefinition> {
        // Only include tools useful for building software
        self.tools
            .tool_definitions_for(&[
                "shell",
                "read_file",
                "write_file",
                "list_dir",
                "apply_patch",
                "http", // For fetching docs/deps
            ])
            .await
    }

    /// Create the system prompt for the build agent.
    fn build_system_prompt(&self, requirement: &BuildRequirement) -> String {
        let mut prompt = format!(
            r#"You are a software developer building a program.

## Task
Build: {name}
Description: {description}
Type: {software_type:?}
Language: {language:?}

## Process
1. Create the project structure with necessary files
2. Implement the code based on the requirements
3. Build/compile if needed
4. Run tests to verify correctness
5. Fix any errors and iterate

## Guidelines
- Write clean, well-structured code
- Handle errors appropriately
- Add minimal but useful comments
- Follow idiomatic patterns for the language
- Test edge cases

## Tools Available
- shell: Run build commands, tests, install dependencies
- read_file: Read existing files
- write_file: Create new files
- apply_patch: Edit existing files surgically
- list_dir: Explore project structure
"#,
            name = requirement.name,
            description = requirement.description,
            software_type = requirement.software_type,
            language = requirement.language,
        );

        // Add tool-specific context when building WASM tools
        if requirement.software_type == SoftwareType::WasmTool {
            prompt.push_str(&self.wasm_tool_context());
        }

        prompt
    }

    /// Get additional context for building WASM tools.
    fn wasm_tool_context(&self) -> String {
        r#"

## WASM Tool Requirements

You are building a WASM Component tool for an autonomous agent using the WASM Component Model.
The tool MUST use `wit_bindgen` and `cargo-component` to build.

## Available Host Functions (from WIT interface)

The host provides these functions via `near::agent::host`:

```rust
// Logging (always available)
host::log(level: LogLevel, message: &str);  // LogLevel: Trace, Debug, Info, Warn, Error

// Time (always available)
host::now_millis() -> u64;  // Unix timestamp in milliseconds

// Workspace (if capability granted)
host::workspace_read(path: &str) -> Option<String>;

// HTTP (if capability granted)
host::http_request(method: &str, url: &str, headers_json: &str, body: Option<Vec<u8>>)
    -> Result<HttpResponse, String>;
// HttpResponse has: status: u16, headers_json: String, body: Vec<u8>

// Tool invocation (if capability granted)
host::tool_invoke(alias: &str, params_json: &str) -> Result<String, String>;

// Secrets (if capability granted) - can only CHECK existence, not read values
host::secret_exists(name: &str) -> bool;
```

## Project Structure

```
my_tool/
├── Cargo.toml
├── wit/
│   └── tool.wit      # Copy from agent's wit/tool.wit
└── src/
    └── lib.rs
```

## Cargo.toml Template

```toml
[package]
name = "my_tool"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.41"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

## src/lib.rs Template

```rust
// Generate bindings from the WIT interface
wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "wit/tool.wit",
});

use serde::{Deserialize, Serialize};
use exports::near::agent::tool::{Guest, Request, Response};
use near::agent::host::{self, LogLevel};

// Your input/output types
#[derive(Deserialize)]
struct MyInput {
    // Define parameters here
}

#[derive(Serialize)]
struct MyOutput {
    // Define output here
}

struct MyTool;

impl Guest for MyTool {
    fn execute(req: Request) -> Response {
        // Parse input
        let input: MyInput = match serde_json::from_str(&req.params) {
            Ok(i) => i,
            Err(e) => return Response {
                output: None,
                error: Some(format!("Invalid input: {}", e)),
            },
        };

        host::log(LogLevel::Info, &format!("Processing request..."));

        // Your implementation here
        let output = MyOutput { /* ... */ };

        // Return success
        Response {
            output: Some(serde_json::to_string(&output).unwrap()),
            error: None,
        }
    }

    fn schema() -> String {
        serde_json::json!({
            "type": "object",
            "properties": {
                // Define your JSON Schema here
            },
            "required": []
        }).to_string()
    }

    fn description() -> String {
        "Description of what this tool does".to_string()
    }
}

export!(MyTool);
```

## Build Commands

```bash
# Install cargo-component (one time)
cargo install cargo-component

# Build the WASM component
cargo component build --release

# Output: target/wasm32-wasip2/release/my_tool.wasm
```

## Capabilities File (my_tool.capabilities.json)

Create alongside the .wasm file to grant capabilities:

```json
{
    "http": {
        "allowed_endpoints": [
            {"host": "api.example.com", "path_prefix": "/v1/"}
        ]
    },
    "workspace": true,
    "secrets": {
        "allowed": ["API_KEY"]
    }
}
```

## Important Notes

1. NEVER panic - always return Response with error field set
2. Secrets are NEVER exposed to WASM - use placeholders like `{API_KEY}` in URLs
   and the host will inject the real value
3. HTTP requests are rate-limited and only allowed to endpoints in capabilities
4. Keep the tool focused on one thing - small, composable tools are better

"#
        .to_string()
    }

    /// Execute the build loop.
    async fn execute_build_loop(
        &self,
        requirement: &BuildRequirement,
        project_dir: &Path,
    ) -> Result<BuildResult, AgentToolError> {
        let build_id = Uuid::new_v4();
        let started_at = Utc::now();
        let mut logs = Vec::new();
        let mut iteration = 0;

        // Create reasoning engine
        let reasoning = Reasoning::new(self.llm.clone(), self.safety.clone());

        // Build initial context
        let tool_defs = self.get_build_tools().await;
        let mut reason_ctx = ReasoningContext::new().with_tools(tool_defs);

        // Add system prompt
        reason_ctx
            .messages
            .push(ChatMessage::system(self.build_system_prompt(requirement)));

        // Add initial user message - directive to force immediate tool use
        reason_ctx.messages.push(ChatMessage::user(format!(
            "Build the {} in directory: {}\n\n\
             Requirements:\n- {}\n\n\
             IMPORTANT: Use the write_file tool NOW to create Cargo.toml. \
             Do not explain, plan, or output JSON—immediately call write_file.",
            requirement.name,
            project_dir.display(),
            requirement.description
        )));

        logs.push(BuildLog {
            timestamp: Utc::now(),
            phase: BuildPhase::Analyzing,
            message: "Starting build process".into(),
            details: None,
        });

        // Main build loop
        let mut current_phase = BuildPhase::Scaffolding;
        let mut last_error: Option<String> = None;
        let mut tools_executed = false;
        let mut consecutive_text_responses = 0;

        loop {
            iteration += 1;

            if iteration > self.config.max_iterations {
                logs.push(BuildLog {
                    timestamp: Utc::now(),
                    phase: BuildPhase::Failed,
                    message: "Maximum iterations exceeded".into(),
                    details: last_error.clone(),
                });

                return Ok(BuildResult {
                    build_id,
                    requirement: requirement.clone(),
                    artifact_path: project_dir.to_path_buf(),
                    logs,
                    success: false,
                    error: Some("Maximum iterations exceeded".into()),
                    started_at,
                    completed_at: Utc::now(),
                    iterations: iteration,
                    validation_warnings: Vec::new(),
                    tests_passed: 0,
                    tests_failed: 0,
                    registered: false,
                });
            }

            // Refresh tool definitions each iteration
            reason_ctx.available_tools = self.get_build_tools().await;

            // Get response from LLM (may be text or tool calls)
            let result = reasoning
                .respond_with_tools(&reason_ctx)
                .await
                .map_err(|e| {
                    AgentToolError::BuilderFailed(format!("LLM response failed: {}", e))
                })?;

            match result {
                RespondResult::Text(response) => {
                    reason_ctx.messages.push(ChatMessage::assistant(&response));

                    // If tools haven't been executed yet, we're stuck in planning mode
                    if !tools_executed {
                        consecutive_text_responses += 1;

                        // Fail fast after 2 consecutive text-only responses
                        if consecutive_text_responses >= 2 {
                            logs.push(BuildLog {
                                timestamp: Utc::now(),
                                phase: BuildPhase::Failed,
                                message: "Builder stuck in planning mode".into(),
                                details: Some(format!(
                                    "LLM returned {} consecutive text responses without calling tools. \
                                     Try a more specific requirement.",
                                    consecutive_text_responses
                                )),
                            });

                            return Ok(BuildResult {
                                build_id,
                                requirement: requirement.clone(),
                                artifact_path: project_dir.to_path_buf(),
                                logs,
                                success: false,
                                error: Some(
                                    "LLM not executing tools - stuck in planning mode".into(),
                                ),
                                started_at,
                                completed_at: Utc::now(),
                                iterations: iteration,
                                validation_warnings: Vec::new(),
                                tests_passed: 0,
                                tests_failed: 0,
                                registered: false,
                            });
                        }

                        tracing::debug!(
                            "Builder: no tools executed (text response #{}/2), forcing tool use",
                            consecutive_text_responses
                        );
                        reason_ctx.messages.push(ChatMessage::user(
                            "STOP. Do NOT output text, JSON specs, or explanations. \
                             Call the write_file tool RIGHT NOW to create Cargo.toml. \
                             Just call the tool—no commentary.",
                        ));
                        continue;
                    }

                    // Reset counter when tools have been executed (we're in completion phase)
                    consecutive_text_responses = 0;

                    // Check for completion signals
                    let response_lower = response.to_lowercase();
                    if response_lower.contains("build complete")
                        || response_lower.contains("successfully built")
                        || response_lower.contains("all tests pass")
                        || response_lower.contains("complete")
                    {
                        logs.push(BuildLog {
                            timestamp: Utc::now(),
                            phase: BuildPhase::Complete,
                            message: "Build completed successfully".into(),
                            details: Some(response),
                        });

                        // Determine artifact path
                        let artifact_path = self.find_artifact(requirement, project_dir).await;

                        return Ok(BuildResult {
                            build_id,
                            requirement: requirement.clone(),
                            artifact_path,
                            logs,
                            success: true,
                            error: None,
                            started_at,
                            completed_at: Utc::now(),
                            iterations: iteration,
                            validation_warnings: Vec::new(),
                            tests_passed: 0,
                            tests_failed: 0,
                            registered: false,
                        });
                    }

                    // Ask for next steps
                    reason_ctx
                        .messages
                        .push(ChatMessage::user("Continue with the next step."));
                }
                RespondResult::ToolCalls(tool_calls) => {
                    tools_executed = true;

                    // Execute each tool call
                    for tc in tool_calls {
                        logs.push(BuildLog {
                            timestamp: Utc::now(),
                            phase: current_phase,
                            message: format!("Executing: {}", tc.name),
                            details: Some(format!("{:?}", tc.arguments)),
                        });

                        // Execute tool
                        let tool_result = self
                            .execute_build_tool(&tc.name, &tc.arguments, project_dir)
                            .await;

                        match tool_result {
                            Ok(output) => {
                                let output_str = serde_json::to_string_pretty(&output.result)
                                    .unwrap_or_default();

                                // Add to context
                                reason_ctx.messages.push(ChatMessage::tool_result(
                                    &tc.id,
                                    &tc.name,
                                    output_str.clone(),
                                ));

                                // Update phase based on tool
                                current_phase = match tc.name.as_str() {
                                    "write_file" => BuildPhase::Implementing,
                                    "shell" if tc.arguments.to_string().contains("build") => {
                                        BuildPhase::Building
                                    }
                                    "shell" if tc.arguments.to_string().contains("test") => {
                                        BuildPhase::Testing
                                    }
                                    _ => current_phase,
                                };

                                // Check for build/test errors in output
                                if output_str.to_lowercase().contains("error:")
                                    || output_str.to_lowercase().contains("error[")
                                    || output_str.to_lowercase().contains("failed")
                                {
                                    last_error = Some(output_str);
                                    current_phase = BuildPhase::Fixing;
                                }
                            }
                            Err(e) => {
                                let error_msg = format!("Tool error: {}", e);
                                last_error = Some(error_msg.clone());

                                reason_ctx.messages.push(ChatMessage::tool_result(
                                    &tc.id,
                                    &tc.name,
                                    format!("Error: {}", e),
                                ));

                                logs.push(BuildLog {
                                    timestamp: Utc::now(),
                                    phase: BuildPhase::Fixing,
                                    message: "Tool execution failed".into(),
                                    details: Some(error_msg),
                                });

                                current_phase = BuildPhase::Fixing;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Execute a build tool.
    async fn execute_build_tool(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
        _project_dir: &Path,
    ) -> Result<ToolOutput, ToolError> {
        let tool =
            self.tools.get(tool_name).await.ok_or_else(|| {
                ToolError::ExecutionFailed(format!("Tool not found: {}", tool_name))
            })?;

        // Execute with a dummy context (build tools don't need job context)
        let ctx = JobContext::default();
        tool.execute(params.clone(), &ctx).await
    }

    /// Find the build artifact based on project type.
    async fn find_artifact(&self, requirement: &BuildRequirement, project_dir: &Path) -> PathBuf {
        match (&requirement.software_type, &requirement.language) {
            (SoftwareType::WasmTool, Language::Rust) => {
                // WASM output location
                project_dir.join(format!(
                    "target/wasm32-wasip2/release/{}.wasm",
                    requirement.name.replace('-', "_")
                ))
            }
            (SoftwareType::CliBinary, Language::Rust) => project_dir.join(format!(
                "target/release/{}",
                requirement.name.replace('-', "_")
            )),
            (SoftwareType::Script, Language::Python) => {
                project_dir.join(format!("{}.py", requirement.name))
            }
            (SoftwareType::Script, Language::Bash) => {
                project_dir.join(format!("{}.sh", requirement.name))
            }
            _ => project_dir.to_path_buf(),
        }
    }
}

#[async_trait]
impl SoftwareBuilder for LlmSoftwareBuilder {
    async fn analyze(&self, description: &str) -> Result<BuildRequirement, AgentToolError> {
        // Use LLM to parse the description
        let reasoning = Reasoning::new(self.llm.clone(), self.safety.clone());

        let prompt = format!(
            r#"Analyze this software requirement and extract structured information.

Description: {}

IMPORTANT: If this is a "tool" that the agent will use (e.g., "calendar tool", "email tool",
"API client tool"), you MUST use:
- software_type: "wasm_tool"
- language: "rust"

Only use cli_binary/script/library for software meant for human end-users, not agent tools.

Respond with a JSON object containing:
- name: A short identifier (snake_case)
- description: What the software should do
- software_type: One of "wasm_tool", "cli_binary", "library", "script", "web_service"
  (PREFER "wasm_tool" for agent-usable tools)
- language: One of "rust", "python", "typescript", "javascript", "go", "bash"
  (PREFER "rust" for wasm_tool)
- input_spec: Expected input format (optional)
- output_spec: Expected output format (optional)
- dependencies: List of external dependencies needed
- capabilities: For WASM tools, list needed capabilities (http, workspace, secrets)

JSON:"#,
            description
        );

        let ctx = ReasoningContext::new().with_message(ChatMessage::user(&prompt));

        let response = reasoning
            .respond(&ctx)
            .await
            .map_err(|e| AgentToolError::BuilderFailed(format!("Analysis failed: {}", e)))?;

        // Extract JSON from response
        let json_start = response.find('{').unwrap_or(0);
        let json_end = response.rfind('}').map(|i| i + 1).unwrap_or(response.len());
        let json_str = &response[json_start..json_end];

        serde_json::from_str(json_str).map_err(|e| {
            AgentToolError::BuilderFailed(format!("Failed to parse requirement: {}", e))
        })
    }

    async fn build(&self, requirement: &BuildRequirement) -> Result<BuildResult, AgentToolError> {
        // Create project directory
        let project_dir = self.config.build_dir.join(&requirement.name);
        if project_dir.exists() {
            std::fs::remove_dir_all(&project_dir).map_err(|e| {
                AgentToolError::BuilderFailed(format!("Failed to clean project dir: {}", e))
            })?;
        }
        std::fs::create_dir_all(&project_dir).map_err(|e| {
            AgentToolError::BuilderFailed(format!("Failed to create project dir: {}", e))
        })?;

        // Run the build loop with timeout
        let result = tokio::time::timeout(
            self.config.timeout,
            self.execute_build_loop(requirement, &project_dir),
        )
        .await;

        match result {
            Ok(Ok(build_result)) => Ok(build_result),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(AgentToolError::BuilderFailed("Build timed out".into())),
        }
    }

    async fn repair(
        &self,
        result: &BuildResult,
        error: &str,
    ) -> Result<BuildResult, AgentToolError> {
        // Create a new requirement with repair context
        let mut requirement = result.requirement.clone();
        requirement.description = format!(
            "{}\n\nPrevious build failed with error:\n{}\n\nFix the issues and rebuild.",
            requirement.description, error
        );

        // Rebuild (preserving project directory if it exists)
        self.build(&requirement).await
    }
}

/// Tool that allows the agent to build software on demand.
pub struct BuildSoftwareTool {
    builder: Arc<dyn SoftwareBuilder>,
}

impl BuildSoftwareTool {
    pub fn new(builder: Arc<dyn SoftwareBuilder>) -> Self {
        Self { builder }
    }
}

#[async_trait]
impl Tool for BuildSoftwareTool {
    fn name(&self) -> &str {
        "build_software"
    }

    fn description(&self) -> &str {
        "Build software from a description. IMPORTANT: For tools the agent will use, \
         ALWAYS build Rust WASM tools (type: wasm_tool, language: rust). Only use cli_binary, \
         script, or other types for software meant for human users. The builder scaffolds, \
         implements, compiles, and tests iteratively."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "Natural language description of what to build"
                },
                "type": {
                    "type": "string",
                    "enum": ["wasm_tool", "cli_binary", "library", "script"],
                    "description": "Type of software to build (optional, will be inferred)"
                },
                "language": {
                    "type": "string",
                    "enum": ["rust", "python", "typescript", "bash"],
                    "description": "Programming language to use (optional, will be inferred)"
                }
            },
            "required": ["description"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let description = params
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("missing 'description'".into()))?;

        let start = std::time::Instant::now();

        // Analyze the requirement
        let mut requirement = self
            .builder
            .analyze(description)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Analysis failed: {}", e)))?;

        // Override type/language if specified
        if let Some(type_str) = params.get("type").and_then(|v| v.as_str()) {
            requirement.software_type = match type_str {
                "wasm_tool" => SoftwareType::WasmTool,
                "cli_binary" => SoftwareType::CliBinary,
                "library" => SoftwareType::Library,
                "script" => SoftwareType::Script,
                _ => requirement.software_type,
            };
        }

        if let Some(lang_str) = params.get("language").and_then(|v| v.as_str()) {
            requirement.language = match lang_str {
                "rust" => Language::Rust,
                "python" => Language::Python,
                "typescript" => Language::TypeScript,
                "bash" => Language::Bash,
                _ => requirement.language,
            };
        }

        // Build
        let result = self
            .builder
            .build(&requirement)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Build failed: {}", e)))?;

        let output = serde_json::json!({
            "build_id": result.build_id.to_string(),
            "name": result.requirement.name,
            "success": result.success,
            "artifact_path": result.artifact_path.display().to_string(),
            "iterations": result.iterations,
            "error": result.error,
            "phases": result.logs.iter().map(|l| format!("{:?}: {}", l.phase, l.message)).collect::<Vec<_>>()
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self) -> bool {
        true // Building software should require approval
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_extensions() {
        assert_eq!(Language::Rust.extension(), "rs");
        assert_eq!(Language::Python.extension(), "py");
        assert_eq!(Language::TypeScript.extension(), "ts");
    }

    #[test]
    fn test_build_commands() {
        assert!(Language::Rust.build_command("/tmp/project").is_some());
        assert!(Language::Python.build_command("/tmp/project").is_none());
    }

    #[test]
    fn test_software_type_serialization() {
        let json = serde_json::to_string(&SoftwareType::WasmTool).unwrap();
        assert_eq!(json, "\"wasm_tool\"");
    }
}
