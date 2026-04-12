# Live Canary Local and GitHub Setup

This directory contains the unified entrypoints for the live regression lanes:

- `run.sh` dispatches named lanes and writes artifacts
- `scrub-artifacts.sh` scans artifacts before upload
- `upgrade-canary.sh` checks previous-release DB compatibility

The auth-focused Python runners remain the executors behind the auth lanes:

- `scripts/auth_canary/run_canary.py`
- `scripts/auth_live_canary/run_live_canary.py`
- `scripts/auth_browser_canary/run_browser_canary.py`

Run commands from the repository root.

## Lane Families

### Upstream live LLM lanes

- `deterministic-replay`
- `public-smoke`
- `persona-rotating`
- `private-oauth`
- `provider-matrix`
- `release-public-full`
- `upgrade-canary`

### Auth lanes added on this branch

- `auth-smoke`
- `auth-full`
- `auth-channels`
- `auth-live-seeded`
- `auth-browser-consent`

## Local Commands

Run the public live smoke lane:

```bash
LANE=public-smoke scripts/live-canary/run.sh
```

Run the provider matrix lane:

```bash
LANE=provider-matrix \
PROVIDER=openai-compatible \
PROVIDER_TEST_TARGET=e2e_live_mission \
SCENARIO=mission_daily_news_digest_with_followup \
scripts/live-canary/run.sh
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

Run selected auth provider cases:

```bash
LANE=auth-live-seeded CASES=gmail,github scripts/live-canary/run.sh
LANE=auth-browser-consent CASES=google,github scripts/live-canary/run.sh
```

Use CI-style browser installation for auth browser lanes:

```bash
LANE=auth-browser-consent PLAYWRIGHT_INSTALL=with-deps scripts/live-canary/run.sh
```

Reuse an existing build and Python environment:

```bash
LANE=auth-smoke SKIP_BUILD=1 SKIP_PYTHON_BOOTSTRAP=1 scripts/live-canary/run.sh
```

Run an upgrade canary:

```bash
LANE=upgrade-canary \
PREVIOUS_REF=v0.1.2 \
CURRENT_REF=HEAD \
scripts/live-canary/run.sh
```

Artifacts are written under:

```text
artifacts/live-canary/<lane>/<provider>/<timestamp>/
```

## Secrets And Account Material

Public live LLM lane secrets and variables are documented in
[docs/internal/live-canary.md](/home/illia/ironclaw/docs/internal/live-canary.md).

Seeded auth live-provider credentials:

- [scripts/auth_live_canary/ACCOUNTS.md](/home/illia/ironclaw/scripts/auth_live_canary/ACCOUNTS.md)

Browser-consent account sessions, OAuth app credentials, and storage-state files:

- [scripts/auth_browser_canary/ACCOUNTS.md](/home/illia/ironclaw/scripts/auth_browser_canary/ACCOUNTS.md)

## GitHub Workflow

GitHub Actions uses `.github/workflows/live-canary.yml` as the single scheduled
and manual entrypoint. That workflow now contains both the upstream live LLM
jobs and the auth-specific canary jobs.
