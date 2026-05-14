# LOCAL_ONLY_CHANGES — t3-claw (`errol/payroll-v2-claw-1`)

**[LOCAL ONLY — DO NOT MERGE]**

## Files to KEEP (real PR content)

### `src/tools/mcp/client.rs` (committed in `94c17ed4`)
`inject_t3n_delegation_credential` reads the per-user
`t3n_delegation_token` secret (a JSON object with
`credential_jcs`/`user_sig`/`agent_pubkey` produced by the trinity
FE) and merges `credential_jcs_b64u` and `user_sig_b64u` into the
arguments of every `t3n-mcp` tool call. Required for the agent path
through the `runPayroll` tool to work.

## Files explicitly NOT on this branch

### `src/tools/mcp/unix_transport.rs` — split to `errol/mcp-unix-transport-reconnect`
Production-shaped reconnect-on-EPIPE patch. It survives review and
ships on its own focused PR off `main` because (a) the use case
(sidecar restart while bastionclaw stays up) is operational, not
runbook-specific, and (b) bundling it with the delegation-injection
fix muddies both PRs' review boundaries.

## Files to REVERT before any real PR

These are reverted on `-1` already.

### `docker-compose.yml`
- bastionclaw host port `3000 → 3300` (collides with trinity leader
  on `3000`).
- Adds `trinity_sdk: ../trinity/client/t3n-sdk` build context for the
  sidecar.
- Adds `T3N_MCP_AGENT_SECRET_HEX` and `T3N_MCP_PRIVATE_KEY` env
  passthrough to the `t3n-mcp-sidecar` service.

### `docker/t3n-mcp-sidecar.Dockerfile`
- Bundles trinity's `t3n-sdk` into the image at `/t3n-sdk` and copies
  `config.local.json` so `T3N_SDK_ENV=local` resolves.

Both pair with the runbook in `trinity/PAYROLL_V2_SPINUP.md`.

### Untracked stale snapshots
`src/cli/snapshots/*.snap.new` were stale insta deltas unrelated to
payroll-v2. Deleted on `-1`.

### `.env` (gitignored)
Contains live `SLACK_BOT_TOKEN`, `GITHUB_TOKEN`, `TELEGRAM_BOT_TOKEN`,
`LLM_API_KEY`, `GATEWAY_AUTH_TOKEN`, `SECRETS_MASTER_KEY`,
`T3N_MCP_AGENT_SECRET_HEX`, `T3N_MCP_PRIVATE_KEY`, plus other
runbook secrets. The single-line `.env` entry in `.gitignore` is
load-bearing — do not move or weaken. After the runbook ends,
rotate at minimum `T3N_MCP_AGENT_SECRET_HEX` and
`T3N_MCP_PRIVATE_KEY` since they were passed through scripts/logs
during debug.

## Sister repos

- `Terminal-3/trinity` `errol/payroll-v2-integration-1` (MCP fixes,
  helper scripts, runbook docs)
- `Terminal-3/contracts` `errol/mat-1419-eth-auth-map(-1)` (platform fix)
- `t3-apps` `errol/payroll-v2-fe-1` (Tailwind token fix; admin UI
  held back)
