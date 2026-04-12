# Live Canary Local and GitHub Setup

This directory is the unified entrypoint for the canary lanes in this branch.

It contains:

- `run.sh` for lane dispatch and artifact layout
- `scrub-artifacts.sh` for basic artifact scanning before upload

The auth-specific runners remain the executors behind those entrypoints:

- `scripts/auth_canary/run_canary.py`
- `scripts/auth_live_canary/run_live_canary.py`
- `scripts/auth_browser_canary/run_browser_canary.py`

Run commands from the repository root.

## Lanes

- `auth-smoke`
  Mock-backed fresh-machine auth smoke lane.
- `auth-full`
  Larger mock-backed auth regression lane.
- `auth-channels`
  WASM channel auth diagnostic lane.
- `auth-live-seeded`
  Real-provider lane that seeds known-good credentials into a clean DB and verifies runtime use and refresh.
- `auth-browser-consent`
  Real provider-consent lane that starts from an empty DB and completes OAuth in Playwright.

## Local Commands

Run the default smoke lane:

```bash
LANE=auth-smoke scripts/live-canary/run.sh
```

Run the seeded live lane:

```bash
LANE=auth-live-seeded scripts/live-canary/run.sh
```

Run the browser-consent lane:

```bash
LANE=auth-browser-consent scripts/live-canary/run.sh
```

Run only selected provider cases:

```bash
LANE=auth-live-seeded CASES=gmail,github scripts/live-canary/run.sh
LANE=auth-browser-consent CASES=google,github scripts/live-canary/run.sh
```

Use CI-style browser installation:

```bash
LANE=auth-browser-consent PLAYWRIGHT_INSTALL=with-deps scripts/live-canary/run.sh
```

Reuse an existing build and Python environment:

```bash
LANE=auth-smoke SKIP_BUILD=1 SKIP_PYTHON_BOOTSTRAP=1 scripts/live-canary/run.sh
```

Artifacts are written under:

```text
artifacts/live-canary/<lane>/<provider>/<timestamp>/
```

## Account Material

Seeded live-provider credentials:

- [scripts/auth_live_canary/ACCOUNTS.md](/home/illia/ironclaw/scripts/auth_live_canary/ACCOUNTS.md)

Browser-consent account sessions, OAuth app credentials, and storage-state files:

- [scripts/auth_browser_canary/ACCOUNTS.md](/home/illia/ironclaw/scripts/auth_browser_canary/ACCOUNTS.md)

## GitHub Workflow

GitHub Actions uses `.github/workflows/live-canary.yml` as the single scheduled
and manual entrypoint. That workflow fans out to the existing auth lanes rather
than maintaining separate workflow files per auth runner.
