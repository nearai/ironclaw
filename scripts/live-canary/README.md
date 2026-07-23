# Live Canary Local and GitHub Setup

This directory contains the unified entrypoints for the live regression lanes:

- `run.sh` dispatches named lanes and writes artifacts
- `scrub-artifacts.sh` scans artifacts before upload
- `upgrade-canary.sh` checks previous-release DB compatibility

The auth-focused Python runners remain the executors behind the auth lanes:

- `scripts/auth_canary/run_canary.py` — mock-backed pytest matrix (fresh-machine)
- `scripts/auth_live_canary/run_live_canary.py` — live-provider runner with two
  modes: `--mode seeded` (token persistence and refresh) and `--mode browser`
  (OAuth consent in Playwright)

Their shared auth canary setup, provider registry, and runtime helpers live in:

- `scripts/live_canary/common.py`
- `scripts/live_canary/auth_registry.py`
- `scripts/live_canary/auth_runtime.py`

Note on naming: `live-canary/` (this directory, hyphen) is the shell dispatcher
and operator-facing entrypoint; `live_canary/` (sibling, underscore) is the
Python package. The hyphen/underscore split follows Python's package-naming
convention — Python imports cannot contain hyphens.

Future auth providers should be added through the shared registry and account
guide, not by creating a new standalone runner shape.

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

### IronClaw WebUI v2 QA lane

- `ironclaw-webui-v2-live-qa`

PR-targeted runs execute the reviewed PR binary with live integration secrets.
They must pass the `reborn-live-canary-pr` GitHub environment gate and have an
approving review for the exact PR head commit from a collaborator with write
access. Scheduled and manual default-branch runs do not require this PR gate.

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
LANE=auth-browser-consent CASES=google,notion scripts/live-canary/run.sh
# Browser cases: google, notion only. github is PAT-only (not OAuth) so
# it lives in auth-live-seeded instead — see scripts/live_canary/auth_registry.py.
```

Run the IronClaw WebUI v2 live QA lane against the local copied IronClaw home:

```bash
LANE=ironclaw-webui-v2-live-qa \
IRONCLAW_WEBUI_V2_LIVE_QA_HOME=/tmp/ironclaw-real-slack \
scripts/live-canary/run.sh
```

Run the full QA-sheet-backed IronClaw suite:

```bash
LANE=ironclaw-webui-v2-live-qa CASES=all scripts/live-canary/run.sh
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

Before upload, strict scrubbing removes only bundled system-skill copies whose
managed marker, stable content hash, file set, and bytes match the
source-controlled bundle from the tested commit. Unverified or unmanaged system
skills and all other run-specific artifacts remain present and are scanned for
secret material. Non-strict scrubbing is report-only and does not prune them.
Strict scrubbing also removes source-byte-verified first-party extension
manifests, whose static credential schema fields otherwise look like live
secrets. The dynamically rendered NEAR AI manifest is instead verified against
a trusted runtime template after normalizing only the repository-owned
`cloud-api.near.ai` and `private.near.ai` MCP endpoints. Changed or unrecognized
manifests remain subject to the fail-closed scanner.

## Secrets And Account Material

Public live LLM lane secrets and variables are documented in
[docs/internal/live-canary.md](../../docs/internal/live-canary.md).

Seeded auth live-provider credentials:

- [scripts/live-canary/ACCOUNTS.md](ACCOUNTS.md)

## GitHub Workflow

GitHub Actions uses `.github/workflows/live-canary.yml` as the single scheduled
and manual entrypoint. That workflow now contains both the upstream live LLM
jobs and the auth-specific canary jobs.
