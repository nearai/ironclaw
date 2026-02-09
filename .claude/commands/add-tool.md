---
description: Scaffold a new tool (WASM or built-in Rust) with all boilerplate wired up
allowed-tools: Read, Edit, Write, Glob, Grep, Bash(cargo fmt:*), Bash(cargo clippy:*), Bash(cargo test:*), Bash(cargo component:*), Bash(ls:*), Bash(mkdir:*)
argument-hint: <tool_name> [description]
model: opus
---

Scaffold a new tool called `$ARGUMENTS` for the IronClaw agent. First, determine the tool type and then follow the appropriate path.

## Step 0: Determine tool type

Ask the user which type of tool to create:

- **WASM tool** (recommended) - Sandboxed, dynamically loadable, external API integrations. Lives in `tools-src/wasm-tools/<name>/`. This is the right choice for anything that talks to an external service (Notion, GitHub, Discord, etc.).
- **Built-in tool** - Compiled into the main binary. Only for core agent infrastructure (e.g., memory, file ops, shell). Lives in `src/tools/builtin/<name>.rs`.

If the description clearly implies an external service integration, default to WASM. If it's a core agent capability, default to built-in.

---

## Path A: WASM Tool

### A1: Create directory structure

Create `tools-src/wasm-tools/<name>/` with:

```
tools-src/wasm-tools/<name>/
├── Cargo.toml
├── <name>-tool.capabilities.json
└── src/
    ├── lib.rs
    ├── types.rs
    └── api.rs
```

### A2: Write `Cargo.toml`

Follow this exact pattern (adjust name and description):

```toml
[package]
name = "<name>-tool"
version = "0.1.0"
edition = "2021"
description = "<Description> tool for IronClaw (WASM component)"
license = "MIT OR Apache-2.0"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[profile.release]
opt-level = "s"
lto = true
strip = true
codegen-units = 1
```

### A3: Write `<name>-tool.capabilities.json`

Declare the tool's security requirements. Determine what APIs it needs and create the allowlist. Reference `tools-src/wasm-tools/slack/slack-tool.capabilities.json` for the format.

Key sections to include:
- `http.allowlist` - API endpoints (host, path_prefix, methods)
- `http.credentials` - Secret injection config (secret_name, location type: bearer/header/query)
- `http.rate_limit` - requests_per_minute, requests_per_hour
- `http.timeout_secs`
- `secrets.allowed_names` - Which secrets the tool can check existence of
- `auth` - Authentication setup (OAuth or manual token entry)

If the tool needs OAuth, include:
```json
{
  "auth": {
    "secret_name": "<service>_token",
    "display_name": "<Service>",
    "oauth": {
      "authorization_url": "https://...",
      "token_url": "https://...",
      "client_id_env": "<SERVICE>_OAUTH_CLIENT_ID",
      "client_secret_env": "<SERVICE>_OAUTH_CLIENT_SECRET",
      "scopes": [],
      "use_pkce": false
    },
    "env_var": "<SERVICE>_TOKEN"
  }
}
```

If no OAuth, include manual setup instructions:
```json
{
  "auth": {
    "secret_name": "<service>_api_key",
    "display_name": "<Service>",
    "instructions": "Get your API key from <url>",
    "setup_url": "https://...",
    "token_hint": "Starts with '<prefix>'",
    "env_var": "<SERVICE>_API_KEY"
  }
}
```

### A4: Write `src/types.rs`

Define the action enum using serde's tagged enum pattern:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum <Name>Action {
    // Add variants based on the tool's capabilities.
    // Each variant maps to one API operation.
}
```

Add result structs with `#[derive(Debug, Serialize)]`. Use `#[serde(skip_serializing_if = "Option::is_none")]` for optional fields.

### A5: Write `src/api.rs`

Implement the API calls using the host HTTP capability:

```rust
use crate::near::agent::host;
use crate::types::*;

const API_BASE: &str = "https://api.example.com";

fn api_call(method: &str, endpoint: &str, body: Option<&str>) -> Result<String, String> {
    let url = format!("{}/{}", API_BASE, endpoint);
    let headers = if body.is_some() {
        r#"{"Content-Type": "application/json"}"#
    } else {
        "{}"
    };
    let body_bytes = body.map(|b| b.as_bytes().to_vec());

    host::log(host::LogLevel::Debug, &format!("API: {} {}", method, endpoint));

    let response = host::http_request(method, &url, headers, body_bytes.as_deref())?;

    if response.status < 200 || response.status >= 300 {
        return Err(format!(
            "API returned status {}: {}",
            response.status,
            String::from_utf8_lossy(&response.body)
        ));
    }

    String::from_utf8(response.body).map_err(|e| format!("Invalid UTF-8: {}", e))
}
```

Add one function per action variant that calls `api_call` and parses the response into the result structs.

### A6: Write `src/lib.rs`

Wire everything together:

```rust
mod api;
mod types;

use types::<Name>Action;

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../../wit/tool.wit",
});

struct <Name>Tool;

impl exports::near::agent::tool::Guest for <Name>Tool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(result) => exports::near::agent::tool::Response {
                output: Some(result),
                error: None,
            },
            Err(e) => exports::near::agent::tool::Response {
                output: None,
                error: Some(e),
            },
        }
    }

    fn schema() -> String {
        // Return JSON Schema matching the action enum
        todo!("Fill in JSON Schema")
    }

    fn description() -> String {
        "<Description>".to_string()
    }
}

fn execute_inner(params: &str) -> Result<String, String> {
    // Check required secrets
    if !crate::near::agent::host::secret_exists("<secret_name>") {
        return Err("<Secret> not configured. Please add the '<secret_name>' secret.".to_string());
    }

    let action: <Name>Action =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {}", e))?;

    crate::near::agent::host::log(
        crate::near::agent::host::LogLevel::Info,
        &format!("Executing action: {:?}", action),
    );

    let result = match action {
        // Dispatch to api:: functions for each variant
    };

    Ok(result)
}

export!(<Name>Tool);
```

Fill in the `schema()` with a proper JSON Schema using `oneOf` for each action variant. Reference `tools-src/wasm-tools/slack/src/lib.rs` for the exact pattern.

### A7: Verify

Run `cargo fmt` in the tool directory. If `cargo-component` is available, run `cargo component build --release` to verify the WASM compiles.

---

## Path B: Built-in Tool

### B1: Create the tool file

Create `src/tools/builtin/<name>.rs` implementing the `Tool` trait:

```rust
use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput};

pub struct <Name>Tool;

#[async_trait]
impl Tool for <Name>Tool {
    fn name(&self) -> &str {
        "<snake_case_name>"
    }

    fn description(&self) -> &str {
        "<Description>"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                // Define parameters here
            },
            "required": []
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        // Extract and validate parameters
        // Do the work
        // Return result

        Ok(ToolOutput::text("result", start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Set true if tool processes external data
    }

    fn requires_approval(&self) -> bool {
        false // Set true if tool is destructive or contacts external services
    }
}
```

If the tool needs shared state (HTTP client, config), add a struct field and `new()` constructor:

```rust
pub struct <Name>Tool {
    client: reqwest::Client,
}

impl <Name>Tool {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client"),
        }
    }
}
```

### B2: Update `src/tools/builtin/mod.rs`

Add the module declaration and pub use, keeping alphabetical order:

```rust
mod <name>;
pub use <name>::<Name>Tool;
```

### B3: Update `src/tools/registry.rs`

Add the import to the `use crate::tools::builtin::{...}` block and register the tool in the appropriate registration method:

- If it's a core tool: add to `register_builtin_tools()`
- If it needs shared state (workspace, context_manager, etc.): create a new `register_<category>_tools()` method or add to an existing one
- Wire the new registration call in `src/main.rs` if a new method was created

### B4: Add tests

Add a `mod tests {}` block at the bottom of the tool file:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::JobContext;

    fn test_context() -> JobContext {
        JobContext::test_default()
    }

    #[tokio::test]
    async fn test_<name>_basic() {
        let tool = <Name>Tool::new();
        let params = serde_json::json!({ /* test params */ });
        let result = tool.execute(params, &test_context()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_<name>_missing_params() {
        let tool = <Name>Tool::new();
        let params = serde_json::json!({});
        let result = tool.execute(params, &test_context()).await;
        assert!(matches!(result, Err(ToolError::InvalidParameters(_))));
    }
}
```

### B5: Quality gate

Run `cargo fmt` and `cargo clippy --all --benches --tests --examples --all-features`. Fix any issues.

Run the new tests: `cargo test --lib -- builtin::<name>::tests`

---

## Checklist

Before finishing, verify:
- [ ] Tool type chosen (WASM or built-in) and confirmed with user
- [ ] All files created with correct structure
- [ ] For WASM: capabilities.json declares all needed permissions (HTTP, secrets, auth)
- [ ] For WASM: JSON Schema in `schema()` matches the action enum variants
- [ ] For built-in: mod.rs updated with module + pub use
- [ ] For built-in: registry.rs imports and registers the tool
- [ ] For built-in: tests added and passing
- [ ] `cargo fmt` clean
- [ ] `cargo clippy` clean (for built-in) or `cargo component build` clean (for WASM)
