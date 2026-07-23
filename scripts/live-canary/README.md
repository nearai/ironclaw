# Live Canary Local and GitHub Setup

This directory contains the unified entrypoints for the retained live
regression lanes:

- `run.sh` dispatches named lanes and writes artifacts
- `scrub-artifacts.sh` scans artifacts before upload
- `upgrade-canary.sh` checks previous-release DB compatibility

The deleted auth and workflow runners are mapped to their replacement
serve-backed coverage in [MIGRATION.md](MIGRATION.md). Do not add a standalone
Python live-runner shape; extend the retained WebUI v2 QA launcher or an owned
Rust integration lane.

Note on naming: `live-canary/` (this directory, hyphen) is the shell dispatcher
and operator-facing entrypoint; `live_canary/` (sibling, underscore) is the
Python package. The hyphen/underscore split follows Python's package-naming
convention — Python imports cannot contain hyphens.

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

### Reborn WebUI v2 QA lane

- `reborn-webui-v2-live-qa`

This is the retained Python live lane. It launches the shipping
`ironclaw serve` binary with an isolated home and workspace, loopback-only
listener, bounded readiness check, child-only credentials, and captured logs.

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

Run the Reborn WebUI v2 live QA lane against the local copied Reborn home:

```bash
LANE=reborn-webui-v2-live-qa \
REBORN_WEBUI_V2_LIVE_QA_HOME=/tmp/ironclaw-reborn-real-slack \
scripts/live-canary/run.sh
```

Run the full QA-sheet-backed Reborn suite:

```bash
LANE=reborn-webui-v2-live-qa CASES=all scripts/live-canary/run.sh
```

Use CI-style browser installation:

```bash
LANE=reborn-webui-v2-live-qa PLAYWRIGHT_INSTALL=with-deps scripts/live-canary/run.sh
```

Reuse an existing build and Python environment:

```bash
LANE=reborn-webui-v2-live-qa SKIP_BUILD=1 SKIP_PYTHON_BOOTSTRAP=1 scripts/live-canary/run.sh
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

## Secrets

Public live LLM lane secrets and variables are documented in
[docs/internal/live-canary.md](../../docs/internal/live-canary.md).

## GitHub Workflow

GitHub Actions uses `.github/workflows/live-canary.yml` as the single scheduled
and manual entrypoint. Retired jobs are absent from the dispatcher and workflow.
