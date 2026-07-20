# v1 Retirement CI and Release Retarget Plan

Tracking issue: #6077

## Purpose

This plan records the live inventory and safe order for moving validation,
packaging, and deployment assumptions onto the canonical Reborn `ironclaw`
binary before the legacy root runtime is deleted.

## Current Reborn Paths

These paths already build or exercise `ironclaw` and should become the
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
| Legacy Docker image | Retired from the default `Dockerfile`; the default image now builds and runs `ironclaw`. |
| Worker image | The published `ironclaw-worker` tag now uses `Dockerfile.process-sandbox`; `Dockerfile.worker` remains only for frozen v1 paths. |
| Test image | `Dockerfile.test` remains for the legacy local-test workflow until that workflow is retired or replaced. |
| Legacy browser E2E | `.github/workflows/e2e.yml` uploads and executes `target/debug/ironclaw-legacy`. |
| Legacy Rust matrix | `.github/workflows/test.yml` is the root package test matrix, now frozen in workflow docs. |
| Build helper | `scripts/build-all.sh` reports `target/release/ironclaw`. |
| Scope classifiers | `.github/workflows/test.yml`, `platform-and-compat.yml`, and `scripts/ci/classify-test-scope.sh` still route selected changes through legacy `src/` and gateway paths. |
| Release packaging | Compile-only preflight: root package `ironclaw` still owns `ironclaw-v*`, so artifact and image publishing remain blocked while the canonical binary belongs to package `ironclaw_reborn_cli`. |

## Retarget Sequence

1. Keep the canonical executable name `ironclaw` and current WebUI crate path
   (`crates/ironclaw_webui`) across build and Docker inputs.
2. Make the default `Dockerfile` the Reborn production image while keeping
   `Dockerfile.reborn` as a temporary compatibility path.
3. Replace legacy E2E gating with Reborn WebUI / Reborn E2E coverage for
   behavior that still matters.
4. Delete or freeze the root package test matrix only after the migration read
   path no longer depends on root `ironclaw`.
5. Update scope classifiers so changes to Reborn-owned crates trigger Reborn
   gates, and deleted paths do not trigger stale jobs.
6. Transfer Cargo package name/version ownership, release-plz tags, cargo-dist,
   and Windows packaging to Reborn together; do not enable only one layer.
7. Remove scripts and docs that still advertise `ironclaw-legacy` as the
   product binary.

## Required Checks Before Deleting Legacy CI

Run these checks in the retargeting PRs, not only in the final deletion PR:

```bash
cargo build -p ironclaw_reborn_cli --features libsql,postgres,inmemory-turn-state --bin ironclaw
cargo test -p ironclaw_architecture
bash scripts/reborn-e2e-rust.sh
git diff --check
```

For Docker/release changes, also run:

```bash
docker build --target runtime -t ironclaw-test:ci .
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
