---
description: Retired v1 SSE scaffold — redirects to the Reborn projection/streaming path (full rewrite pending)
allowed-tools: Read, Glob, Grep
argument-hint: <event_name> [description]
model: opus
---

# This command's scaffold procedure was retired with the v1 codebase

The step-by-step procedure this command used to carry scaffolded into the
deleted v1 gateway SSE path (`src/channels/`, `crates/ironclaw_gateway/static/`,
`src/agent/` / `src/worker/`). The `ironclaw_gateway` crate and the entire root
`src/` monolith were deleted from the tree; none of those files exist and none
should be created. The dead steps were removed rather than left behind a
warning banner (2026-08-05 stale-docs sweep; the paths were first found dead in
PR #6944, which scoped the rewrite out).

**The Reborn replacement procedure has not been written.** A correct rewrite
targets the `ironclaw_webui` streaming path — the Reborn projection/SSE frame
served by `crates/product/ironclaw_webui`, with the client side in
`crates/product/ironclaw_webui/frontend/`, over the event-stream substrate
(`crates/events/ironclaw_event_log` → `crates/events/ironclaw_event_projections`
→ `crates/events/ironclaw_event_streams`). This stub deliberately does not
guess at the step list.

Until the rewrite lands, to add a new user-visible event `$ARGUMENTS`:

1. Start from the `reborn-feature` skill (`.claude/skills/reborn-feature/SKILL.md`)
   to wire the feature across the layers.
2. Read `.claude/rules/gateway-events.md` — the live Reborn events and
   transport-projection rules.
3. Find the current server-side stream seam with
   `grep -n "stream_events" crates/product/ironclaw_webui/src/webui_v2/handlers.rs`
   and the client consumption in `crates/product/ironclaw_webui/frontend/src/`.
