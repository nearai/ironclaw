---
name: tool-builder
version: "0.1.0"
description: Build and install a user-authored WASM tool with manifest-first approval and inline progress checkpoints
activation:
  keywords:
    - "build a tool"
    - "make a tool"
    - "create a tool"
    - "tool for"
    - "custom tool"
    - "user-authored tool"
  patterns:
    - "(?i)(build|make|create)\s+(me\s+)?a\s+tool"
    - "(?i)i need a tool for"
    - "(?i)custom tool"
  tags:
    - "development"
    - "tools"
  max_context_tokens: 1800
requires:
  bins: [cargo, cargo-component]
---

# Tool Builder

Use this skill when the user wants IronClaw to create a new WASM tool from code and then register it into the system.

## Goal

Produce a **locally-built WASM tool** and install it through `tool_install` with:
- `name`
- `kind: "wasm_tool"`
- `wasm_path`
- `manifest` (the full capabilities JSON object)

The `manifest` is the approval surface. Keep it honest, minimal, and human-readable.

## Required workflow

1. Create or update the source directory.
2. Build the WASM artifact with `cargo component build --release`.
3. Prepare a minimal manifest/capabilities JSON.
4. Emit a `tool-builder` progress block for the current phase.
5. Only after the manifest is ready, call `tool_install` with `wasm_path` and `manifest`.

## OAuth guardrail

If the target integration requires OAuth client registration or a browser OAuth flow:
- do **not** keep building blindly.
- tell the user that user-authored OAuth tools are not supported in this flow yet.
- recommend the closest registry tool if one exists.
- stop before `tool_install` unless the API also supports a manual token/API key path.

## Progress UI contract

After each major phase, emit a fenced block exactly like this:

```tool-builder
{
  "stage": "scaffold",
  "status": "running",
  "title": "Scaffolding tool",
  "name": "my_tool",
  "summary": "Created Cargo.toml, wit/tool.wit, and src/lib.rs"
}
```

Allowed `stage` values:
- `scaffold`
- `implement`
- `build`
- `manifest`
- `install`
- `done`
- `blocked`

Allowed `status` values:
- `running`
- `ready`
- `blocked`
- `error`

Optional fields:
- `artifact_path`
- `manifest`
- `notes`
- `next_step`

When the manifest is ready, include it inline:

```tool-builder
{
  "stage": "manifest",
  "status": "ready",
  "title": "Manifest ready for approval",
  "name": "my_tool",
  "summary": "Prepared a minimal allowlist and setup instructions",
  "manifest": {
    "description": "What the tool does",
    "http": {
      "allowlist": [
        { "host": "api.example.com", "path_prefix": "/v1/", "methods": ["GET"] }
      ]
    },
    "setup": {
      "required_secrets": [
        {
          "name": "example_api_key",
          "prompt": "Paste your Example API key from https://example.com/settings/api"
        }
      ]
    },
    "auth": {
      "provider": "Example",
      "secret_name": "example_api_key",
      "instructions": "Generate an API key in Example settings and paste it into the setup form.",
      "setup_url": "https://example.com/settings/api"
    }
  }
}
```

## Manifest rules

- Minimize permissions.
- Prefer exact hosts and path prefixes.
- Prefer manual token/API-key auth over broad secret scopes.
- Always include a clear `description`.
- If setup is required, include `setup.required_secrets` and `auth` guidance.
- Do not request capabilities the code does not need.

## Install call shape

Use this exact pattern once the artifact exists:

```json
{
  "name": "my_tool",
  "kind": "wasm_tool",
  "wasm_path": "/absolute/or/relative/path/to/target/wasm32-wasip2/release/my_tool.wasm",
  "manifest": { ...capabilities json... }
}
```

## Response style

- Keep prose short.
- Let the progress blocks carry the structured state.
- If blocked, emit a `tool-builder` block with `stage: "blocked"` and explain why.
