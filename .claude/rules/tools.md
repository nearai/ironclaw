---
paths:
  - "src/tools/**"
  - "tools-src/**"
  - "src/channels/**"
  - "src/cli/**"
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

## Everything Goes Through Tools

**All actions originating from any non-agent caller — gateway handlers, CLI
commands, routine engine, WASM channels, future channel extensions — MUST
go through `ToolDispatcher::dispatch()`, never directly through the
database, workspace, or domain managers.**

This is the core design principle behind #2049. The reasons are concrete:

1. **Audit trail.** Every dispatched call creates an `ActionRecord` linked
   to a system job, so UI-initiated mutations are visible in job history
   alongside agent-initiated ones. Direct DB calls bypass this entirely.
2. **Safety pipeline parity.** The dispatcher runs the same pipeline as
   `Worker::execute_tool`: parameter normalization, schema validation,
   `sensitive_params()` redaction, per-tool timeout, output sanitization.
   Direct calls skip all of it and risk leaking secrets into logs or
   persisting unsafe content.
3. **Channel-agnostic.** Channels are interchangeable extensions (gateway,
   CLI, telegram, WASM, future custom channels). Routing through a single
   dispatch function means new channels inherit the full pipeline for free.
4. **Agent parity.** The agent can do anything channels can do (and vice
   versa), because both call the same tools. No more "the UI can install
   extensions but the agent can only list them" gaps.

### Required pattern

```rust
// In any gateway handler, CLI command, or routine engine callback:
use crate::tools::dispatch::{DispatchSource, ToolDispatcher};

let dispatcher: &ToolDispatcher = state
    .tool_dispatcher
    .as_ref()
    .ok_or((StatusCode::SERVICE_UNAVAILABLE, "dispatcher unavailable"))?;

let output = dispatcher
    .dispatch(
        "memory_write",
        serde_json::json!({ "target": path, "content": content }),
        &user.user_id,
        DispatchSource::Channel("gateway".into()),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
```

### Forbidden pattern

```rust
// DO NOT do this in a gateway handler, CLI command, or routine callback:
let store = state.store.as_ref().ok_or(...)?;
store.set_setting(&user.user_id, &key, &value).await?;  // BYPASSES dispatch

let workspace = resolve_workspace(&state, &user).await?;
workspace.write(path, content).await?;  // BYPASSES dispatch + safety pipeline

let ext_mgr = state.extension_manager.as_ref().ok_or(...)?;
ext_mgr.install(name, url, kind, &user.user_id).await?;  // BYPASSES audit trail
```

### When direct access IS allowed

The dispatch principle applies to **non-agent callers** acting on behalf of
a user. These are exempt:

| Layer | Why exempt |
|---|---|
| `Worker::execute_tool()` (agent loop) | Has its own atomic sequence-numbered audit trail; the dispatcher would conflict |
| `EffectBridgeAdapter::execute_action()` (v2 engine) | Same — its own audit via `ThreadEvent` event sourcing |
| The tool implementations themselves | Tools are the leaves; they need direct `Workspace`, `Database`, etc. handles to do their work |
| Background jobs (scheduler, hygiene, mission runner) inside the engine | These ARE the engine; they emit their own events |
| Pure read endpoints that need to JOIN/aggregate from multiple sources | A single tool call cannot express "list all jobs across users with filters X, Y, Z" — these are queries, not actions, and the audit value is low |

### Annotating intentional exceptions

If a handler legitimately needs direct access (rare — usually only for
read aggregation), suppress the pre-commit check with a trailing comment
on the offending line:

```rust
let rows = state.store.list_agent_jobs().await?; // dispatch-exempt: read-only aggregation
```

The pre-commit hook (`scripts/pre-commit-safety.sh`) flags any newly
added line in `src/channels/web/handlers/*.rs` or `src/cli/*.rs` that
touches `state.{store,workspace,workspace_pool,extension_manager,
skill_registry,session_manager}.*` without a trailing
`// dispatch-exempt: <reason>` comment on the same line. The check only
looks at added lines (`+` lines in the diff), so existing untouched code
doesn't trip it during incremental migration.

### Migration status

As of #2049, `ToolDispatcher` is wired into `GatewayState` but per-handler
migration is incomplete. New handlers MUST use the dispatcher. Existing
handlers should be migrated incrementally; each handler family
(settings, memory, extensions, skills, routines, jobs, threads) is its
own follow-up PR.
