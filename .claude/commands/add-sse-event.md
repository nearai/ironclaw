---
description: Add a Reborn event projection and WebUI stream consumer
allowed-tools: Read, Edit, Write, Glob, Grep, Bash(cargo fmt:*), Bash(cargo clippy:*), Bash(cargo test:*)
argument-hint: <event_name> [description]
model: opus
---

Add `$ARGUMENTS` to the supported Reborn event path. Do not create a parallel
transport state machine.

1. Locate the owning durable event or projection contract under `crates/`.
2. Carry the typed event through `ironclaw_event_projections` and
   `ironclaw_event_streams`; transports must not invent authoritative state.
3. Expose the projection through the descriptor, router, and handler owned by
   `crates/ironclaw_webui/src/webui_v2/`.
4. Consume the stream in `crates/ironclaw_webui/frontend/src/` using the
   existing API and state-management patterns.
5. Add a contract test at the projection/handler seam and a frontend test for
   the rendered state. Test the caller that emits the event when it gates a
   side effect.
6. Run the owning crate tests, WebUI tests, workspace formatting, and clippy.

Before editing, use `bash scripts/codebase-graph.sh status` and verify the live
symbols with targeted `rg`; the retired root `src/` gateway is not a valid
implementation target.
