# t3-claw / errol/payroll-v2-claw-1 — branch notes

**[LOCAL ONLY — DO NOT MERGE AS-IS]**

## What's on this branch

Predecessor: `errol/payroll-v2-claw` (1 commit ahead of `origin/main`).

Two commits ahead of main:

1. `94c17ed4` — `feat(mcp): inject Trinity delegation credential into
   t3n-mcp tool calls` (KEEP / SHIP). Already on the predecessor.
2. `[LOCAL] docs: branch notes …` — drop before any real PR.

## What's NOT on this branch

- `src/tools/mcp/unix_transport.rs` reconnect-on-EPIPE patch. Split
  to `errol/mcp-unix-transport-reconnect` off `origin/main`.
- `docker-compose.yml` and `docker/t3n-mcp-sidecar.Dockerfile`
  payroll-v2 dev plumbing. Reverted to HEAD's tracked content.
- Stale `src/cli/snapshots/*.snap.new` files (deleted; unrelated to
  payroll-v2).

## To produce a real PR

1. `git rebase -i origin/main` and drop the `[LOCAL]` commit.
2. The remaining commit (`94c17ed4`) is the PR.

## Sister branches

- `errol/payroll-v2-claw` (predecessor)
- `errol/mcp-unix-transport-reconnect` (split-out transport fix off main)
- `Terminal-3/trinity` `errol/payroll-v2-integration-1`
- `Terminal-3/contracts` `errol/mat-1419-eth-auth-map(-1)`
- `t3-apps` `errol/payroll-v2-fe-1`
