# Live Canary Regression Lanes

IronClaw has two complementary regression systems:

- deterministic CI, which replays committed LLM traces without network calls;
- live canaries, which use real LLM providers and selected real tools to catch
  provider drift, prompt/tool orchestration drift, OAuth expiry, and release
  upgrade problems.

The implementation lives in:

- `.github/workflows/test.yml` for the blocking replay lane;
- `.github/workflows/live-canary.yml` for scheduled and manual live lanes;
- `scripts/live-canary/run.sh` for lane dispatch;
- `scripts/live-canary/scrub-artifacts.sh` for artifact scanning;
- `scripts/live-canary/upgrade-canary.sh` for previous-release upgrade checks.

For local commands and GitHub secret setup, see
`scripts/live-canary/README.md`.

## Lane Summary

| Lane | Scope | Runner | Trigger | Blocking |
| --- | --- | --- | --- | --- |
| `deterministic-replay` | Replays `tests/e2e_live*.rs` fixtures without live LLM calls | GitHub-hosted | PR/staging via `test.yml`; manual via `live-canary.yml` | Yes in `test.yml` |
| `public-smoke` | Real LLM plus public tools: `zizmor_scan` and mission digest | GitHub-hosted | Daily and manual | Opens issue on scheduled failure |
| `persona-rotating` | Real LLM multi-turn persona workflow, one persona per day | GitHub-hosted | Daily and manual | Opens issue on scheduled failure |
| `private-oauth` | Google Drive auth gate and transparent refresh against a dedicated test account | Self-hosted `ironclaw-live` runner | Manual; scheduled only when enabled | Opens issue on scheduled failure |
| `provider-matrix` | Same live behavior against multiple provider adapters | GitHub-hosted | Weekly and manual | Opens issue on scheduled failure |
| `release-public-full` | Full public live suite for release candidates | GitHub-hosted | Manual | Release checklist gate |
| `upgrade-canary` | Previous release DB opened by current checkout | GitHub-hosted | Manual | Release checklist gate |

## Required Repository Configuration

Secrets:

- `LIVE_ANTHROPIC_API_KEY`: API key for the default public live lanes.
- `LIVE_OPENAI_COMPATIBLE_API_KEY`: provider-matrix key for the OpenAI-compatible lane.
- `LIVE_OPENAI_COMPATIBLE_BASE_URL`: provider-matrix base URL, for example an OpenRouter, LiteLLM, or internal gateway URL.

Variables:

- `LIVE_ANTHROPIC_MODEL`: optional; defaults to `claude-sonnet-4-6`.
- `LIVE_OPENAI_COMPATIBLE_MODEL`: required when the OpenAI-compatible provider lane is enabled.
- `LIVE_CANARY_PRIVATE_OAUTH_ENABLED`: set to `true` only after the self-hosted runner is ready.

Keep the public live keys scoped to a budget-limited account. These lanes are
scheduled and intentionally exercise multi-turn agent behavior.

## Runner Setup

### GitHub-hosted public lanes

No persistent state is required. The workflow sets:

```bash
DATABASE_BACKEND=libsql
LIBSQL_PATH=${RUNNER_TEMP}/ironclaw-live-*.db
ALLOW_LOCAL_TOOLS=true
AGENT_AUTO_APPROVE_TOOLS=true
IRONCLAW_LIVE_TEST=1
```

The live harness starts from a clean libSQL database. Tests that need state must
seed it in the test body.

### Self-hosted private OAuth lane

Use a dedicated runner with labels:

```text
self-hosted
ironclaw-live
```

Provision a dedicated test Google account, not a maintainer account. On that
runner, run IronClaw once manually and complete Google Drive auth so the local
profile has these secrets:

- `google_oauth_token`
- `google_oauth_token_refresh_token`
- `google_oauth_token_scopes`

The `private-oauth` lane calls `with_no_trace_recording()` in the underlying
tests. Do not upload raw logs from this lane. The workflow uploads only summary
files and runs the strict artifact scrubber before upload.

## Commands

Replay locally without LLM calls:

```bash
LANE=deterministic-replay scripts/live-canary/run.sh
```

Run public live smoke locally:

```bash
IRONCLAW_LIVE_TEST=1 \
LLM_BACKEND=anthropic \
ANTHROPIC_API_KEY=... \
LANE=public-smoke \
scripts/live-canary/run.sh
```

Run a specific persona:

```bash
IRONCLAW_LIVE_TEST=1 \
LLM_BACKEND=anthropic \
ANTHROPIC_API_KEY=... \
LANE=persona-rotating \
SCENARIO=developer_full_workflow \
scripts/live-canary/run.sh
```

Run private OAuth on the dedicated runner:

```bash
LANE=private-oauth scripts/live-canary/run.sh
```

Run an upgrade canary:

```bash
LANE=upgrade-canary \
PREVIOUS_REF=v0.1.2 \
CURRENT_REF=HEAD \
scripts/live-canary/run.sh
```

## Persona Rotation

The rotating persona lane uses UTC day of week:

| Day | Scenario |
| --- | --- |
| Monday | `ceo_full_workflow` |
| Tuesday | `content_creator_full_workflow` |
| Wednesday | `trader_full_workflow` |
| Thursday | `developer_full_workflow` |
| Friday | `developer_full_workflow` |
| Saturday | `ceo_full_workflow` |
| Sunday | `content_creator_full_workflow` |

Override with `SCENARIO=<test-name>` or workflow dispatch input `scenario`.

## Failure Handling

Scheduled live failures create GitHub issues with the run URL and commit SHA.
Treat these as canary findings, not automatic release blockers, until the lane
has passed consistently for two weeks.

Release candidates should manually run:

1. `release-public-full`
2. `private-oauth` on the dedicated runner
3. `provider-matrix`
4. `upgrade-canary`

Do not override a release live gate without writing the failure mode and rollback
risk in the release issue or PR.

## Trace and Artifact Policy

Live mode can update files under `tests/fixtures/llm_traces/live/`. CI does not
commit those files. If a developer intentionally re-records traces, follow the
PII scrub checklist in `tests/support/LIVE_TESTING.md` before committing them.

Artifact uploads must come from `artifacts/live-canary/`, not from
`tests/fixtures/llm_traces/live/`. Private OAuth lanes must not upload raw logs
or trace files.
