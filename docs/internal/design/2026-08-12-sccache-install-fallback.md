# Best-Effort sccache Installation

## Problem

The shared `setup-sccache-dist` composite action makes every opted-in CI lane depend on downloading an `sccache` release asset. The upstream installer retries transient download failures, but after those retries are exhausted the lane fails before its tests start. This failed two independent runs for PR #7477 and ultimately removed it from the merge queue even though `sccache` is only a compilation-cache optimization.

## Design

Keep the existing pinned installer and its built-in retries, but mark only that installer step as best-effort. Give the step the ID `install_sccache` so later composite-action steps can inspect its pre-`continue-on-error` outcome.

When installation succeeds, preserve the current behavior: validate the cache and distributed-compiler settings, establish the SSH tunnel, write the configuration, set `RUSTC_WRAPPER=sccache`, and probe the scheduler. Configuration and connection failures remain fatal because they indicate an IronClaw-owned setup defect after the binary is available.

When installation fails, emit a warning, skip all cache configuration, do not set `RUSTC_WRAPPER`, and let the caller continue with ordinary local Rust compilation. No additional retry layer is added because the upstream action already retries the release download.

## Verification

Extend the existing CI workflow-contract validator and sabotage tests to pin three properties: the installer is best-effort, configuration requires a successful installer outcome, and a failed installer produces a local-compilation warning. Because every Reborn test job consumes the shared action, map its directory to the exhaustive Reborn PR test plan and retain fail-closed handling for other unmapped local actions. Run the affected-area planner tests, focused workflow-contract tests, the checked-in workflow validator, `actionlint` when available, and the docs publication-boundary check.

## Compatibility and rollback

Successful cache setup is unchanged. The failure path is slower because it compiles locally, but it no longer turns an optional cache outage into a false test failure. Pull requests that change this shared action run the exhaustive Reborn plan, increasing CI cost only for this rare CI-infrastructure change class. Rollback is a revert of the composite-action, workflow-contract, and affected-area planner changes.
