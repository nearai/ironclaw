# LOCAL_ONLY_CHANGES — t3-claw (`errol/payroll-v2-claw-1`)

**[LOCAL ONLY — DO NOT MERGE]**

This branch is the **runnable** local dev branch — delegation
credential injection + EPIPE-reconnect transport patch + the docker
plumbing for the sibling trinity checkout. PR-clean variant is
`errol/payroll-v2-claw-1-lite`.

## Files to KEEP (real PR content)

### `src/tools/mcp/client.rs` (in commit `94c17ed4`)
`inject_t3n_delegation_credential` reads the per-user
`t3n_delegation_token` secret and merges `credential_jcs_b64u` +
`user_sig_b64u` into the arguments of every `t3n-mcp` tool call.
Required for the agent path through the `runPayroll` tool.

### `src/tools/mcp/unix_transport.rs` (cherry-picked from `errol/mcp-unix-transport-reconnect`)
Production-shaped reconnect-on-EPIPE patch. The sidecar restarts
independently of bastionclaw during rolling deploys, env-var
rotations, and sidecar rebuilds; the existing transport hangs for
30s per call until bastionclaw is restarted otherwise. Aborts the
stale reader, drains pending waiters, opens a fresh UnixStream,
respawns the reader, single-flight via a reconnect lock, exactly
one retry.

This patch is split onto its own branch `errol/mcp-unix-transport-reconnect`
off `origin/main` for an independent small-scope PR; the same commit
is cherry-picked here for local-dev convenience.

## Files on `-1` for LOCAL ITERATION (NOT on `-1-lite`)

### `docker-compose.yml`
- bastionclaw host port `3000 → 3300` (trinity leader binds host `:3000`).
- `t3n-mcp-sidecar.build.additional_contexts.trinity_sdk: ../trinity/client/t3n-sdk`
  so the Dockerfile can `COPY --from=trinity_sdk`.
- `t3n-mcp-sidecar.environment.T3N_MCP_AGENT_SECRET_HEX: ${T3N_MCP_AGENT_SECRET_HEX:-}`
  env passthrough so the runPayroll handler can sign `agent_sig`.

### `docker/t3n-mcp-sidecar.Dockerfile`
- Bundles trinity's t3n-sdk at `/t3n-sdk` BEFORE running
  `pnpm install` (the mcp `package.json` has a `link:../../t3n-sdk`
  resolved-relative dep that needs the path to exist).
- Copies `config.local.json` so `T3N_SDK_ENV=local` bootstraps
  cleanly. Runtime env vars (`T3N_MCP_RPC_URL`,
  `T3N_MCP_DASHBOARD_URL`) override its values anyway.

## Untracked / ignored

### `.env`
Gitignored at `.gitignore:2`. Contains live `SLACK_BOT_TOKEN`,
`GITHUB_TOKEN`, `TELEGRAM_BOT_TOKEN`, `LLM_API_KEY`,
`GATEWAY_AUTH_TOKEN`, `SECRETS_MASTER_KEY`,
`T3N_MCP_AGENT_SECRET_HEX`, `T3N_MCP_PRIVATE_KEY` and other runbook
secrets. The single-line `.env` entry in `.gitignore` is load-bearing.
After the runbook ends, rotate `T3N_MCP_AGENT_SECRET_HEX` and
`T3N_MCP_PRIVATE_KEY` (they passed through scripts/logs during
debug).

### `.DS_Store` files
macOS Finder noise. Untracked, harmless. Should be added to
`.gitignore` someday.

## How to revert to `-1-lite` shape

1. `git rebase -i origin/main` and drop the `[LOCAL] *` commits +
   the `fix(mcp): reconnect Unix transport …` commit (that one
   continues to live on `errol/mcp-unix-transport-reconnect`).
2. The remaining commit is `94c17ed4` — same as `-1-lite`.

## Sister repos

- `Terminal-3/trinity` `errol/payroll-v2-integration-1`
- `Terminal-3/contracts` `errol/mat-1419-eth-auth-map-1`
- `t3-apps` `errol/payroll-v2-fe-1`
- This repo: `errol/mcp-unix-transport-reconnect` (the cherry-pick source)
