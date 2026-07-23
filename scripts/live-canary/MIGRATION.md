# Live-canary migration inventory

This inventory is the source of truth for the live-canary lanes after the
`ironclaw-legacy` binary was removed. A retained lane either exercises Rust
integration tests directly or starts the shipping `ironclaw serve` binary. A
retired lane is no longer selectable in the dispatcher or GitHub workflow and
names the coverage that replaces it.

| Lane | Disposition | Runtime | Replacement or evidence |
| --- | --- | --- | --- |
| `deterministic-replay` | Retained | Rust integration test | `tests/e2e_live.rs` |
| `public-smoke` | Retained | Rust integration test | `tests/e2e_live.rs`, `tests/e2e_live_mission.rs` |
| `persona-rotating` | Retained | Rust integration test | `tests/e2e_live_personas.rs` |
| `private-oauth` | Retained | Rust integration test | `tests/e2e_live.rs::drive_transparent_oauth_refresh` |
| `provider-matrix` | Retained | Rust integration test | Operator-selected live integration target |
| `release-public-full` | Retained | Rust integration test | Full public live integration set |
| `upgrade-canary` | Retained | Shipping `ironclaw` CLI | `scripts/live-canary/upgrade-canary.sh` |
| `reborn-webui-v2-live-qa` | Retained | Shipping `ironclaw serve` | `scripts/reborn_webui_v2_live_qa/run_live_qa.py` |
| `auth-smoke` | Retired | — | `tests/e2e/scenarios/test_reborn_webui_v2_product_auth_api.py` |
| `auth-full` | Retired | — | `tests/e2e/scenarios/test_v2_auth_oauth_matrix.py` plus the retained-scenario migration tracked by #6561 |
| `auth-channels` | Retired | — | `tests/e2e/scenarios/test_reborn_slack_channel_e2e.py` plus the retained-scenario migration tracked by #6561 |
| `auth-live-seeded` | Retired | — | Product-auth cases in `scripts/reborn_webui_v2_live_qa/run_live_qa.py` |
| `auth-browser-consent` | Retired | — | Product-auth OAuth cases in `scripts/reborn_webui_v2_live_qa/run_live_qa.py` |
| `workflow-canary` | Retired | — | Workflow/routine cases in `scripts/reborn_webui_v2_live_qa/run_live_qa.py` |

The Reborn WebUI v2 runner is the live launcher contract. It creates an
isolated home and workspace per run, binds loopback only, injects credentials
only into the child environment, waits on `/api/health`, bounds startup, and
captures stdout/stderr under the lane artifact directory. Its launcher tests
live in `scripts/reborn_webui_v2_live_qa/test_run_live_qa.py`.

The retired Python runners remain temporarily as migration input for #6561 and
are deleted with the rest of the obsolete infrastructure by #6562. They are
not reachable from the operator entrypoint or CI.
