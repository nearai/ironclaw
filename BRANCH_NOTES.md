# t3-claw / errol/payroll-v2-claw-1 — branch notes

**[LOCAL ONLY — DO NOT MERGE AS-IS]**

## Naming convention

| Branch | Purpose |
|---|---|
| `errol/payroll-v2-claw` | predecessor — `94c17ed4` delegation credential injection |
| `errol/payroll-v2-claw-1-lite` | PR-clean — same as predecessor + LOCAL docs |
| `errol/payroll-v2-claw-1` (this branch) | runnable local dev — `-1-lite` + unix_transport patch + docker plumbing |
| `errol/mcp-unix-transport-reconnect` | standalone main-bound PR for the transport patch (off `origin/main`) |

## What's on this branch

5 commits ahead of `origin/main` (newest first):

1. `[LOCAL] docs: branch notes for runnable errol/payroll-v2-claw-1`
2. `[LOCAL] docker: payroll-v2 dev plumbing for sibling trinity checkout`
3. `fix(mcp): reconnect Unix transport on broken-pipe writes` (cherry-picked)
4. `[LOCAL] docs: branch notes for errol/payroll-v2-claw-1` (from `-1-lite`)
5. `94c17ed4 feat(mcp): inject Trinity delegation credential into t3n-mcp tool calls`

## How to use this branch

For a fresh local payroll-v2 stack:

```fish
cd /Users/errol_hava/Documents/Central/Code/terminal3/t3-claw
# Ensure .env has T3N_MCP_PRIVATE_KEY and T3N_MCP_AGENT_SECRET_HEX (see trinity's PAYROLL_V2_SPINUP.md §0)
docker compose --profile app up --build
```

Watch http://localhost:3300.

For a real PR:
- The delegation-injection commit (`94c17ed4`) ships from
  `errol/payroll-v2-claw` or `-1-lite`.
- The transport fix ships from `errol/mcp-unix-transport-reconnect`.

## Sister branches

- `errol/payroll-v2-claw` (predecessor)
- `errol/payroll-v2-claw-1-lite` (PR-clean)
- `errol/mcp-unix-transport-reconnect` (standalone)
- `Terminal-3/trinity` `errol/payroll-v2-integration-1`
- `Terminal-3/contracts` `errol/mat-1419-eth-auth-map-1`
- `t3-apps` `errol/payroll-v2-fe-1`
