---
title: "fix: Prevent Kubernetes pod label validation failures for sandbox jobs"
type: fix
status: active
date: 2026-04-11
origin: docs/brainstorms/2026-04-11-k8s-pod-label-validation-requirements.md
deepened: 2026-04-11
---

# fix: Prevent Kubernetes pod label validation failures for sandbox jobs

## Overview

Sandbox job creation currently stamps `ironclaw.created_at` into
`WorkloadSpec.labels` as an RFC3339 string. Docker accepts that label value,
but Kubernetes rejects it when the label is copied into pod
`metadata.labels`, causing `create_job` to fail with a 422 before the worker
Pod starts.

This plan fixes the write path, keeps readers backward-compatible with old
RFC3339 labels, and adds regression coverage through the actual caller that
writes workload labels.

## Problem Frame

The Kubernetes runtime path is functionally blocked for normal sandbox job
execution because the workload label format is not valid for Kubernetes. The
bug is not a cluster connectivity or RBAC problem; it is a metadata encoding
bug in the `create_job` caller path. Reaper and workload discovery also depend
on this label, so the fix must preserve cleanup semantics and mixed-fleet
compatibility. (See origin:
`docs/brainstorms/2026-04-11-k8s-pod-label-validation-requirements.md`)

## Requirements Trace

- R1. Kubernetes sandbox job creation must stop writing invalid label values.
- R2. `create_job` must succeed for normal workloads when label encoding was
  the only blocker.
- R3. Creation-time metadata must still support orphan detection and age-based
  cleanup semantics.
- R4. Docker and Kubernetes workload readers must accept historical RFC3339
  labels and the new label-safe format.
- R5. Regression coverage must exercise the caller path that writes labels.
- R6. Docker behavior must remain unchanged from the user’s perspective.

## Scope Boundaries

- No redesign of the overall workload labeling scheme beyond making
  `ironclaw.created_at` label-safe.
- No change to namespace selection, runtime selection, kube credential loading,
  or orchestrator URL construction.
- No switch to platform-native timestamps as the canonical source in this fix.
- No new annotations, CRDs, persistence changes, or broader runtime metadata
  refactors.

## Context & Research

### Relevant Code and Patterns

- `ContainerJobManager::create_job()` and `create_job_inner()` in
  `src/orchestrator/job_manager.rs` are the only production write path for
  `ironclaw.created_at`.
- `DockerRuntime::list_managed_workloads()` in `src/sandbox/docker.rs` parses
  RFC3339 from `ironclaw.created_at`, then falls back to Docker's
  `summary.created`.
- `KubernetesRuntime::list_managed_workloads()` in
  `src/sandbox/kubernetes.rs` parses RFC3339 from `ironclaw.created_at`, then
  falls back to `pod.metadata.creation_timestamp`.
- `SandboxReaper::scan_and_reap()` in `src/orchestrator/reaper.rs` consumes
  `ManagedWorkload.created_at`; it does not parse labels directly.
- `ContainerJobManager::with_runtime()` in `src/orchestrator/job_manager.rs`
  provides an existing seam for caller-path tests with an injected runtime
  stub.
- Existing test homes:
  - `src/orchestrator/job_manager.rs` `mod tests`
  - `src/sandbox/runtime.rs` `mod tests`
  - `src/sandbox/kubernetes.rs` `mod tests`
  - `src/sandbox/docker.rs` `mod tests`
  - `src/orchestrator/reaper.rs` `mod tests`

### Institutional Learnings

- `.claude/rules/testing.md` requires testing through the caller when a helper
  or transform gates a side effect through a wrapper. This applies here because
  label formatting sits between `ContainerJobManager::create_job()` and the
  side effecting `create_and_start_workload()` runtime call.

### External References

- None. Local code patterns and the origin requirements doc are sufficient for
  this fix.

## Key Technical Decisions

- **Keep `ironclaw.created_at` as the canonical workload label**: This fix
  stays within the current metadata model instead of moving creation time to a
  different source of truth. That preserves blast radius and matches the origin
  scope boundary.

- **Encode new label values as Unix milliseconds strings**: A decimal
  millisecond timestamp uses only Kubernetes-label-safe characters, stays well
  below label length limits, and round-trips directly to `DateTime<Utc>` with
  no timezone ambiguity.

- **Introduce a shared label codec in the runtime layer**: A small shared
  helper in `src/sandbox/runtime.rs` should format and parse
  `ironclaw.created_at` so Docker, Kubernetes, and orchestrator write/read the
  same rules. This is a scoped deduplication, not a metadata refactor.

- **Broaden readers before or with the writer change**: Readers must accept
  both Unix-millis and historical RFC3339 labels before new writes ship, so
  mixed fleets and old Docker workloads remain discoverable.

- **Parse only one numeric contract**: The new numeric format should be treated
  as Unix milliseconds only, not “any numeric timestamp.” That avoids seconds /
  millis ambiguity and keeps cleanup ordering deterministic.

- **Caller-path regression lives in `job_manager.rs` tests**: The best
  high-signal regression test is an async unit test that drives
  `ContainerJobManager::create_job()` with `with_runtime()` and a recording
  `ContainerRuntime` stub, then asserts on the emitted `WorkloadSpec.labels`.

## Open Questions

### Resolved During Planning

- **Which caller path should own the regression test?** `src/orchestrator/job_manager.rs`
  should host it. It already contains orchestrator-level tests and exposes
  `with_runtime()`, which lets the test drive the real public `create_job()`
  path without Docker or Kubernetes.

- **Should this become an integration test under `tests/`?** No. The bug is a
  label encoding bug on the `create_job()` write path; DB, gateway, and full
  web harnesses add cost without increasing confidence materially.

- **Where should shared timestamp label helpers live?** `src/sandbox/runtime.rs`
  is the best shared home because both runtimes consume the parsing logic and
  orchestrator code already depends on runtime-layer types.

### Deferred to Implementation

- Exact helper naming inside `src/sandbox/runtime.rs`, as long as the helpers
  remain scoped to `ironclaw.created_at` and do not broaden this fix into a
  general metadata abstraction.

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for
> review, not implementation specification. The implementing agent should treat
> it as context, not code to reproduce.*

```text
ContainerJobManager::create_job()
  -> create_job_inner()
  -> format label-safe created_at
  -> WorkloadSpec.labels["ironclaw.created_at"] = "<unix-millis>"
  -> runtime.create_and_start_workload()
     -> Docker: labels copied to container metadata
     -> Kubernetes: labels copied to pod metadata.labels

Runtime readers:
  parse_created_at_label(label):
    1. try unix-millis
    2. else try RFC3339
    3. else None

  Docker list_managed_workloads():
    parsed label -> fallback summary.created

  Kubernetes list_managed_workloads():
    parsed label -> fallback metadata.creation_timestamp

Reaper:
  list_managed_workloads() -> ManagedWorkload.created_at -> age threshold logic
```

## Implementation Units

- [ ] **Unit 1: Add shared codec and broaden readers first**

**Goal:** Teach Docker and Kubernetes workload discovery to accept both the new
label-safe timestamp encoding and historical RFC3339 labels before any writer
switch happens.

**Requirements:** R3, R4, R6

**Dependencies:** None

**Files:**
- Modify: `src/sandbox/runtime.rs`
- Modify: `src/sandbox/docker.rs`
- Modify: `src/sandbox/kubernetes.rs`
- Modify: `src/orchestrator/reaper.rs`
- Test: `src/sandbox/runtime.rs`
- Test: `src/sandbox/docker.rs`
- Test: `src/sandbox/kubernetes.rs`
- Test: `src/orchestrator/reaper.rs`

**Approach:**
- Add a small shared formatter/parser pair for `ironclaw.created_at` in
  `src/sandbox/runtime.rs`, but use only the parser in this unit.
- Reader behavior becomes: parse unix-millis -> parse RFC3339 -> fall back to
  platform-native creation timestamp.
- Keep current fallback order unchanged in each runtime.
- Update `reaper.rs` tests to reflect dual-format expectations; production
  reaper logic should remain unchanged because it consumes `ManagedWorkload`,
  not raw labels.

**Patterns to follow:**
- `src/sandbox/runtime.rs` as the home of shared runtime metadata types
- Existing fallback structure in `src/sandbox/docker.rs`
- Existing fallback structure in `src/sandbox/kubernetes.rs`
- `SandboxReaper::scan_and_reap()` in `src/orchestrator/reaper.rs`

**Test scenarios:**
- Happy path: shared parser accepts unix-millis label values and returns a
  `DateTime<Utc>`.
- Backward compatibility: shared parser still accepts historical RFC3339 label
  values.
- Edge case: malformed label value returns `None`, allowing Docker
  `summary.created` and Kubernetes `metadata.creation_timestamp` fallbacks to
  remain active.
- Happy path: Docker reader resolves `created_at` from new unix-millis labels.
- Happy path: Kubernetes reader resolves `created_at` from new unix-millis
  labels.
- Backward compatibility: reaper-oriented tests still validate old RFC3339
  fixtures and malformed fallback behavior without changing cleanup ordering.

**Verification:**
- `cargo test --all-features -p ironclaw -- sandbox::runtime`
  remains green.
- `cargo test --all-features -p ironclaw -- sandbox::docker`
  remains green.
- `cargo test --all-features -p ironclaw -- sandbox::kubernetes`
  remains green.
- `cargo test --all-features -p ironclaw -- orchestrator::reaper`
  remains green.

- [ ] **Unit 2: Switch the writer and add caller-path regression coverage**

**Goal:** Make `ContainerJobManager::create_job()` emit Kubernetes-safe label
values using the shared formatter, and prove it through the actual side-effect
gate.

**Requirements:** R1, R2, R3, R5

**Dependencies:** Unit 1

**Files:**
- Modify: `src/orchestrator/job_manager.rs`
- Modify: `src/sandbox/runtime.rs`
- Test: `src/orchestrator/job_manager.rs`

**Approach:**
- Update `ContainerJobManager::create_job_inner()` to stamp
  `ironclaw.created_at` using the shared formatter and leave
  `ironclaw.job_id` unchanged.
- Add a caller-path regression in `job_manager.rs` using
  `ContainerJobManager::with_runtime()` and a recording `ContainerRuntime`
  stub that captures the `WorkloadSpec` passed to
  `create_and_start_workload()`.
- Make the regression assert on the actual emitted label string and its
  round-trip through the shared parser.
- Land this unit atomically after Unit 1; do not split the writer switch into a
  separate deploy before readers are dual-format compatible.

**Patterns to follow:**
- `ContainerJobManager::with_runtime()` in `src/orchestrator/job_manager.rs`
- Existing orchestrator-level tests in `src/orchestrator/job_manager.rs`
- Repo testing rule in `.claude/rules/testing.md`

**Test scenarios:**
- Happy path: `create_job()` emits `ironclaw.created_at` using only
  Kubernetes-label-safe characters.
- Happy path: the emitted label round-trips through the shared parser to a
  valid `DateTime<Utc>`.
- Integration: driving the public `create_job()` path reaches
  `create_and_start_workload()` with the formatted label intact.
- Edge case: the regression keeps `ironclaw.job_id` unchanged while switching
  only the timestamp label.
- Edge case: if an additional mode is easy to cover with the existing stub,
  verify the write path does not diverge across job modes.

**Verification:**
- The caller-path regression fails on the old RFC3339 writer and passes with
  the new writer.
- `cargo test --all-features -p ironclaw -- orchestrator::job_manager`
  remains green.

- [ ] **Unit 3: Validate blast radius and update change notes**

**Goal:** Confirm the runtime fix is scoped, documented, and verified at the
right layer before landing.

**Requirements:** R2, R5, R6

**Dependencies:** Units 1, 2

**Files:**
- Modify: `CHANGELOG.md`
- Test: `src/orchestrator/job_manager.rs`
- Test: `src/sandbox/docker.rs`
- Test: `src/sandbox/kubernetes.rs`
- Test: `src/orchestrator/reaper.rs`

**Approach:**
- Add a focused changelog note for the Kubernetes sandbox pod-creation fix.
- Run targeted checks covering the writer path, runtime readers, and reaper
  semantics rather than broad unrelated suites.
- Verify that Docker remains user-visible compatible even if label encoding now
  differs under the hood.
- Call out in the final change notes that mixed binary versions may temporarily
  fall back to platform-native creation timestamps until all readers are on the
  dual-format parser, and that external tooling scraping raw label values may
  need adjustment if it assumed RFC3339 strings.

**Patterns to follow:**
- Existing `CHANGELOG.md` entry style
- Repo testing guidance in `.claude/rules/testing.md`

**Test scenarios:**
- Integration: the `create_job()` caller-path regression plus reader tests
  together prove that the original 422 failure mode is removed and cleanup
  semantics remain intact.
- Edge case: mixed-format labels (old RFC3339 and new unix-millis) coexist in
  the same reader logic without ambiguity because numeric strings are treated as
  Unix milliseconds only.

**Verification:**
- `cargo check --all-features` passes.
- `cargo clippy --all --benches --tests --examples --all-features` passes.
- Targeted runtime/reaper tests pass without regressions.

## System-Wide Impact

- **Interaction graph:** `ContainerJobManager::create_job()` writes
  `WorkloadSpec.labels`, both runtimes copy/read that metadata, and
  `SandboxReaper::scan_and_reap()` consumes the resulting
  `ManagedWorkload.created_at`.
- **Error propagation:** The current Kubernetes 422 pod-creation failure should
  disappear at the source. Reader-side failures remain non-fatal and continue
  to use fallback or skip behavior.
- **State lifecycle risks:** Mixed fleets will temporarily contain both old
  RFC3339 labels and new unix-millis labels. Readers must accept both until old
  workloads naturally age out.
- **Rollout semantics:** If a newer writer lands before every reader process is
  upgraded, readers that have not yet gained dual-format parsing will fall back
  to platform-native timestamps rather than disappearing workloads entirely.
  That can shift cleanup timing slightly during rollout, so Units 1 and 2
  should land atomically in normal development flow.
- **API surface parity:** No public API, tool schema, or environment-variable
  contract changes. The fix is internal to runtime metadata.
- **External consumers:** Any out-of-repo tooling that inspects raw
  `ironclaw.created_at` label strings may see digit-only values after this fix;
  in-repo behavior remains covered by the compatibility reader path.
- **Integration coverage:** Confidence requires the caller-path test in
  `job_manager.rs`; helper-only parser tests are not sufficient under repo
  testing rules.
- **Unchanged invariants:** `ironclaw.job_id` stays unchanged; runtime
  selection, namespace behavior, kube auth loading, and orchestrator URL
  construction remain untouched.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| New label format fixes Kubernetes but breaks discovery of historical Docker workloads | Ship dual-format readers before the writer switch and land Units 1–2 atomically |
| Small differences between label time and platform fallback time alter cleanup timing | Verify threshold ordering rather than exact timestamp equality; preserve existing fallback order |
| Numeric parsing accepts the wrong scale (seconds vs millis) | Parse only Unix-millis numeric strings and document that contract in the shared codec tests |
| Scope drifts into broader metadata redesign | Keep helper changes local to `ironclaw.created_at` and explicitly avoid switching canonical source or adding annotations |

## Documentation / Operational Notes

- Update `CHANGELOG.md` because this changes observable Kubernetes sandbox
  behavior from “pod create fails” to “worker pod starts successfully”.
- No README or config-doc update is required because no user-facing settings or
  setup steps change.

## Sources & References

- **Origin document:** [docs/brainstorms/2026-04-11-k8s-pod-label-validation-requirements.md](docs/brainstorms/2026-04-11-k8s-pod-label-validation-requirements.md)
- Testing guidance: `.claude/rules/testing.md`
- Write path: `src/orchestrator/job_manager.rs` (`create_job`, `create_job_inner`, `with_runtime`)
- Runtime readers: `src/sandbox/docker.rs` (`list_managed_workloads`), `src/sandbox/kubernetes.rs` (`list_managed_workloads`)
- Cleanup consumer: `src/orchestrator/reaper.rs` (`scan_and_reap`)
