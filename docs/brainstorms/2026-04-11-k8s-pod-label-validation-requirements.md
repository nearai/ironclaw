---
date: 2026-04-11
topic: k8s-pod-label-validation
---

# Kubernetes Pod Label Validation Fix

## Problem Frame

Sandbox jobs targeting the Kubernetes runtime can fail before a worker Pod is
created because `ironclaw.created_at` is written as an RFC3339 timestamp into
`metadata.labels`. Kubernetes label values reject characters like `:` and `+`,
so `create_job` returns a 422 validation error instead of starting the worker.

This blocks the newly-added Kubernetes sandbox path for normal job execution
and turns a routine worker launch into a hard failure.

## Requirements

**Pod creation compatibility**
- R1. Sandbox job creation for the Kubernetes runtime must not write label
  values that violate Kubernetes label validation rules.
- R2. The `create_job` path must successfully create a worker Pod for normal
  workloads when cluster connectivity, namespace, image settings, and RBAC are
  otherwise valid, and the only prior failure mode was invalid label encoding.

**Workload metadata continuity**
- R3. Workload metadata must still carry enough creation-time information for
  orphan detection and age-based cleanup, preserving the current age-based
  ordering behavior used by workload listing and reaper logic.
- R4. Docker and Kubernetes workload listing must continue to recover
  `created_at` correctly for workloads created before and after the fix,
  including historical RFC3339-formatted labels where they already exist.

**Regression protection**
- R5. Verification must cover the actual caller path that writes workload
  labels, not only a helper-level parser.
- R6. Existing Docker behavior must remain unchanged from the user’s
  perspective.

## Success Criteria

- A `create_job` request like the reported example no longer fails with a 422
  Kubernetes label validation error.
- Newly created Kubernetes worker Pods appear in the target namespace.
- Reaper / workload listing logic still derives a usable `created_at` value for
  both old-format and new-format labels, without changing cleanup ordering.
- Targeted checks and tests pass without introducing clippy warnings, including
  coverage through the caller path that writes workload labels.

## Scope Boundaries

- No redesign of the overall workload labeling scheme beyond what is needed to
  make `ironclaw.created_at` label-safe.
- No change to namespace selection, runtime selection, or kube credential
  loading.
- No switch to platform-native timestamps as the canonical source in this fix.
- No new pod annotations, CRDs, or persistence features as part of this fix.

## Key Decisions

- Use a Kubernetes-label-safe timestamp representation for
  `ironclaw.created_at`: The current RFC3339 value is invalid for Kubernetes
  labels; the replacement must keep cleanup/reaper semantics while satisfying
  label constraints.
- Keep backward-compatible readers: Docker/Kubernetes workload discovery should
  continue to parse historical RFC3339 label values as long as they may still
  exist on already-created workloads.
- Keep `ironclaw.created_at` as the canonical workload label for this bug fix:
  platform-native timestamps may still be used as fallback readers where they
  already exist, but changing the canonical source is out of scope here.
- Treat this as a runtime bug fix, not a broader metadata refactor: the highest
  leverage move is to unblock worker creation with minimal blast radius.

## Dependencies / Assumptions

- Kubernetes worker Pods continue to use `metadata.labels` for
  `ironclaw.job_id` and creation-time lookup.
- Existing reaper/listing code is the consumer of `ironclaw.created_at`.

## Outstanding Questions

### Deferred to Planning
- [Affects R5][Needs research] Which targeted test best exercises the caller
  path with minimal fixture churn: `ContainerJobManager::create_job` through a
  mocked runtime, or current reaper/runtime listing tests plus a focused
  caller-path unit test?

## Next Steps

→ `/prompts:ce-plan` for structured implementation planning
