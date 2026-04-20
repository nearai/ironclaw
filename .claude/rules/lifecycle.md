---
paths:
  - "src/channels/**"
  - "src/tools/wasm/**"
  - "src/tools/mcp/**"
  - "src/bridge/**"
---
# Discovery vs. Activation

**Installed is not active.** These are distinct states with distinct triggers:

- **Discovery** — boot-time scan that enumerates what the user has installed (WASM channels, MCP servers, extensions). Produces a manifest. Side-effect-free.
- **Activation** — explicit state transition that brings an installed thing into a running state (channel opened, WS connected, hooks registered, credentials bound).

A bug shape has recurred 5× on the WASM channel surface (#2556, #2557, #2558, #2564, #2419): activation-level behavior bound to the discovery scan.

## Rules

1. **Registration of runtime effects happens at activation, not discovery.** Hook registration, websocket spawn, poll task spawn, reconnect loops, long-lived state — none of these may run from `discover()`, `list_installed()`, or boot-time iteration of the manifest.

2. **Auth rejection is terminal until credentials change.** A WASM/MCP channel that fails auth on connect MUST NOT retry in a reconnect loop. It transitions to `AuthFailed` and stays there until a credential-change event (new OAuth token, new bot token) triggers re-activation.

3. **`list_installed` vs. `list_active` are separate queries.** Status surfaces and dispatch paths must use the query that matches their intent. A dashboard saying "N channels" must specify which.

4. **Deactivation unwinds everything activation set up.** When a user disables or uninstalls an extension, every runtime effect must be reversed: hooks removed, WS closed, poll task cancelled, in-flight reconnect aborted, snapshot state dropped. No orphaned tasks.

5. **Discovery is idempotent and side-effect-free.** Repeated discovery scans produce the same manifest and must not start tasks, open connections, or touch the network.

6. **Snapshot rehydrate must re-validate.** When restoring cached state across a restart (pending auth prompts, leases, gate state), re-run the type's `::new()` constructor AND check domain invariants (not-revoked, not-expired, `thread_id` matches). Stale snapshots cause "ghost" leases and replayed auth. References: PR #2617 `restore_selected_auth_prompt`, PR #2631 paused-lease rehydrate.
