# v1 Retirement CI and Release Retarget Plan

Tracking issue: #6077

## Purpose

This plan scopes the retargeting work needed before the legacy root `ironclaw`
runtime can be deleted. It does not change CI or release behavior directly; it
records the live inventory and the safe order for moving validation, packaging,
and deployment assumptions onto `ironclaw-reborn`.

## Current Reborn Paths

These paths already build or exercise `ironclaw-reborn` and should become the
primary references during the retirement:

| Area | Current path |
| --- | --- |
| Reborn crate/unit/integration coverage | `.github/workflows/reborn-tests.yml` |
| Reborn deterministic E2E contracts | `.github/workflows/reborn-e2e.yml`, `scripts/reborn-e2e-rust.sh` |
| Reborn browser/WebUI checks | `.github/workflows/reborn-playwright.yml` |
| Reborn hosted container | `Dockerfile` (default production image), `Dockerfile.reborn` (explicit compatibility path) |
| Reborn local WebUI helper | `scripts/run-reborn-webui.sh` |
| Reborn live canary binary artifact | `.github/workflows/live-canary.yml` |

## Legacy Paths Still Needing Retarget or Deletion

| Area | Current dependency |
| --- | --- |
| Legacy Docker image | Retired from the default `Dockerfile`; the default image now builds and runs `ironclaw-reborn`. |
| Worker image | `Dockerfile.worker` retired because it built and ran the v1 `ironclaw` worker image. |
| Test image | `Dockerfile.test` retired because it built the v1 gateway test image. |
| Legacy browser E2E | `.github/workflows/e2e.yml` uploads and executes `target/debug/ironclaw`. |
| Legacy Rust matrix | `.github/workflows/test.yml` is the root package test matrix, now frozen in workflow docs. |
| Build helper | `scripts/build-all.sh` reports `target/release/ironclaw`. |
| Scope classifiers | `.github/workflows/test.yml`, `platform-and-compat.yml`, and `scripts/ci/classify-test-scope.sh` still route selected changes through legacy `src/` and gateway paths. |
| Release packaging | cargo-dist now builds `ironclaw_reborn_cli` with the Reborn shipping feature set while keeping the existing `ironclaw-v*` tag family. |

## Retarget Sequence

1. Confirm `ironclaw-reborn` has the intended production packaging flags.
2. Retarget Docker deployment to `Dockerfile.reborn`; decide whether the legacy
   `Dockerfile` names should be removed or kept temporarily as compatibility
   stubs.
3. Replace legacy E2E gating with Reborn WebUI / Reborn E2E coverage for
   behavior that still matters.
4. Delete or freeze the root package test matrix only after the migration read
   path no longer depends on root `ironclaw`.
5. Update scope classifiers so changes to Reborn-owned crates trigger Reborn
   gates, and deleted paths do not trigger stale jobs.
6. Enable release/installer generation for `ironclaw-reborn` after cargo-dist
   tag/versioning and Windows packaging are resolved.
7. Remove scripts and docs that still advertise `target/release/ironclaw` as the
   product binary.

## Required Checks Before Deleting Legacy CI

Run these checks in the retargeting PRs, not only in the final deletion PR:

```bash
cargo build -p ironclaw_reborn_cli --features openai-compat-beta,slack-v2-host-beta,webui-v2-beta,libsql,postgres,inmemory-turn-state --bin ironclaw-reborn
cargo test -p ironclaw_architecture
bash scripts/reborn-e2e-rust.sh
git diff --check
```

For Docker/release changes, also run:

```bash
docker build --target runtime -t ironclaw-reborn-test:ci .
```

## Rollback

Keep each retargeting change isolated by area. If a Reborn CI or packaging path
regresses, revert the narrow retargeting PR and keep the legacy path frozen until
the Reborn replacement is repaired.

## Risk

Risk level: medium.

The direct edits are configuration-heavy, but a missed classifier or packaging
path can silently reduce coverage or ship the wrong binary. Treat merge-queue and
release workflow changes as higher risk than ordinary docs cleanup.
