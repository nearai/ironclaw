# Live Canary Regression Lanes

IronClaw now has two complementary regression systems:

- deterministic CI, which replays committed tests and traces without depending
  on real third-party providers for the main blocking path;
- live canaries, which use real providers, real browser consent flows, or
  selected real LLM lanes to catch provider drift, refresh failures, release
  upgrade problems, and auth regressions that mocks will miss.

The implementation lives in:

- `.github/workflows/test.yml` for the normal blocking test lanes;
- `.github/workflows/live-canary.yml` for scheduled and manual live lanes;
- `scripts/live-canary/run.sh` for lane dispatch;
- `scripts/live-canary/scrub-artifacts.sh` for artifact scanning;
- `scripts/live-canary/upgrade-canary.sh` for previous-release upgrade checks.

The auth-specific executors used by the unified live-canary wrapper are:

- `scripts/auth_canary/run_canary.py`
- `scripts/auth_live_canary/run_live_canary.py`
- `scripts/auth_browser_canary/run_browser_canary.py`

## Lane Summary

| Lane | Scope | Runner | Trigger | Blocking |
| --- | --- | --- | --- | --- |
| `deterministic-replay` | Replays `tests/e2e_live*.rs` fixtures without live LLM calls | GitHub-hosted | PR/staging via `test.yml`; manual via `live-canary.yml` | Yes in `test.yml` |
| `public-smoke` | Real LLM plus public tools such as `zizmor_scan` and mission digest | GitHub-hosted | Daily and manual | Opens issue on scheduled failure |
| `persona-rotating` | Real LLM multi-turn persona workflow, one persona per day | GitHub-hosted | Daily and manual | Opens issue on scheduled failure |
| `private-oauth` | Google Drive auth gate and transparent refresh against a dedicated test account | Self-hosted `ironclaw-live` runner | Manual; scheduled only when enabled | Opens issue on scheduled failure |
| `provider-matrix` | Same live behavior against multiple provider adapters | GitHub-hosted | Weekly and manual | Opens issue on scheduled failure |
| `release-public-full` | Full public live suite for release candidates | GitHub-hosted | Manual | Release checklist gate |
| `upgrade-canary` | Previous release DB opened by current checkout | GitHub-hosted | Manual | Release checklist gate |
| `auth-smoke` | Fresh-machine mock-backed auth smoke: hosted OAuth, MCP OAuth, and multi-user MCP isolation | GitHub-hosted | Hourly and manual | No |
| `auth-full` | Larger mock-backed auth matrix including failure and refresh cases | GitHub-hosted | Manual | No |
| `auth-channels` | WASM channel auth diagnostic lane | GitHub-hosted | Manual | No |
| `auth-live-seeded` | Real-provider runtime checks using seeded tokens against a clean DB | GitHub-hosted | Hourly and manual | No |
| `auth-browser-consent` | Real browser-consent OAuth using Playwright against provider login UIs | GitHub-hosted | Nightly and manual | No |

## Required Repository Configuration

### Public live LLM lanes

Secrets:

- `LIVE_ANTHROPIC_API_KEY`
- `LIVE_OPENAI_COMPATIBLE_API_KEY`
- `LIVE_OPENAI_COMPATIBLE_BASE_URL`

Variables:

- `LIVE_ANTHROPIC_MODEL`
- `LIVE_OPENAI_COMPATIBLE_MODEL`
- `LIVE_CANARY_PRIVATE_OAUTH_ENABLED`

### Auth live-seeded lane

Secrets and dedicated account material are documented in
[scripts/auth_live_canary/ACCOUNTS.md](/home/illia/ironclaw/scripts/auth_live_canary/ACCOUNTS.md).

Current provider material includes:

- Google OAuth client credentials and seeded access/refresh tokens
- GitHub seeded token plus a stable issue fixture
- Notion seeded access token and a stable query fixture

### Auth browser-consent lane

Secrets and browser session material are documented in
[scripts/auth_browser_canary/ACCOUNTS.md](/home/illia/ironclaw/scripts/auth_browser_canary/ACCOUNTS.md).

Current provider material includes:

- Google OAuth app credentials plus browser storage state
- GitHub OAuth app credentials plus browser storage state and issue fixture
- Notion browser storage state

## Commands

Run public live smoke locally:

```bash
IRONCLAW_LIVE_TEST=1 \
LLM_BACKEND=anthropic \
ANTHROPIC_API_KEY=... \
LANE=public-smoke \
scripts/live-canary/run.sh
```

Run a private OAuth lane on the dedicated runner:

```bash
LANE=private-oauth scripts/live-canary/run.sh
```

Run the auth smoke lane:

```bash
LANE=auth-smoke scripts/live-canary/run.sh
```

Run the seeded auth live lane:

```bash
LANE=auth-live-seeded scripts/live-canary/run.sh
```

Run the browser-consent auth lane:

```bash
LANE=auth-browser-consent scripts/live-canary/run.sh
```

Run selected auth provider cases only:

```bash
LANE=auth-live-seeded CASES=gmail,github scripts/live-canary/run.sh
LANE=auth-browser-consent CASES=google,github scripts/live-canary/run.sh
```

## Artifact Policy

Artifacts are written under `artifacts/live-canary/`.

Before upload, the workflow runs `scripts/live-canary/scrub-artifacts.sh`.
That script is a guardrail against uploading obvious token-shaped strings from
logs or result files.

Private OAuth lanes should continue to avoid uploading raw OAuth logs. The
auth-browser-consent and auth-live-seeded lanes may capture screenshots and JSON
results, but should not upload long-lived credential material.
