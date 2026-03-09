---
paths:
  - "src/tools/**"
  - "tools-src/**"
---
# Tool Architecture

**Keep tool-specific logic out of the main agent codebase.** The main agent provides generic infrastructure; tools are self-contained units that declare requirements through `<name>.capabilities.json` sidecar files (in dev mode: `tools-src/<name>/<name>-tool.capabilities.json`).

Tools can be WASM (sandboxed, credential-injected, single binary) or MCP servers (ecosystem, any language, no sandbox). Both are first-class via `ironclaw tool install`.

See `src/tools/README.md` for full architecture, adding new tools, auth JSON examples, and WASM vs MCP decision guide.

## Tool Implementation Pattern

```rust
#[async_trait]
impl Tool for MyTool {
    fn name(&self) -> &str { "my_tool" }
    fn description(&self) -> &str { "Does something useful" }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "param": { "type": "string", "description": "A parameter" }
            },
            "required": ["param"]
        })
    }
    async fn execute(&self, params: serde_json::Value, ctx: &JobContext)
        -> Result<ToolOutput, ToolError>
    {
        let start = std::time::Instant::now();
        // ... do work ...
        Ok(ToolOutput::text("result", start.elapsed()))
    }
    fn requires_sanitization(&self) -> bool { true } // External data
}
```
