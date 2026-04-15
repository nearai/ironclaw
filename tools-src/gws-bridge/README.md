# GWS MCP Bridge

This is a standalone MCP stdio server that wraps a local `gws` binary.
It is meant as an opt-in fallback for users who cannot complete the IC-native
Google OAuth flow.

## What it does

- Exposes a single MCP tool: `gws_bridge`
- Allows only a strict read-only allowlist of commands in phase 1
- Redacts common secret/token patterns from output
- Requires an explicit opt-in environment variable
- Inherits only `PATH` and `HOME` for the wrapped `gws` process

## Build

```bash
cargo build --release
```

## Run

```bash
GWS_BRIDGE_ENABLED=true \
GWS_BINARY_PATH=/path/to/gws \
cargo run --release
```

If `GWS_BINARY_PATH` is omitted, the server looks for `gws` in `PATH`.
The bridge does not forward arbitrary `GWS_*` variables to the child process.

## Register in IronClaw

Use the MCP stdio transport:

```bash
ironclaw mcp add gws-bridge \
  --transport stdio \
  --command /absolute/path/to/gws-bridge \
  --env GWS_BRIDGE_ENABLED=true \
  --env GWS_BINARY_PATH=/absolute/path/to/gws
```

## Allowed phase-1 commands

- `auth status`
- `gmail list`
- `gmail users messages list`
- `calendar events list`
- `calendar users events list`
- `drive files list`

Anything else is rejected.
