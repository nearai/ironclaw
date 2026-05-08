---
date: 2026-04-14
topic: kubernetes-runtime-maturity
---

# Kubernetes Runtime Maturity

## Problem Frame

IronClaw already has the beginnings of a Kubernetes backend, but the current
state mixes together two very different promises: "workers can run on
Kubernetes" and "Kubernetes provides a sandbox experience close to Docker."
That makes scope hard to reason about and makes feature status easy to
overstate.

The product needs a staged path that treats Kubernetes as a Kubernetes-native
runtime, not as a thin Docker imitation. Each stage should make a clear,
truthful promise about what users can rely on, what is still unsupported, and
how the system should behave when a task falls outside that stage.

## Requirements

**Positioning and Guarantees**
- R1. Kubernetes support must be described as a staged maturity track with
  explicit capability boundaries at each stage.
- R2. User-facing language must distinguish "Kubernetes worker runtime" from
  "full sandbox parity" until the final stage is complete.
- R3. Unsupported task types must fail clearly and intentionally, with
  actionable guidance, rather than silently degrading or producing misleading
  success.
- R4. Stage progression must be additive: work enabled in an earlier stage
  remains supported unless it is explicitly replaced by a stricter, clearer
  contract.

**Stage 1: Kubernetes Worker Runtime**
- R5. Stage 1 supports running workers on Kubernetes for tasks that do not
  depend on host bind mounts or host-local network controls.
- R6. Stage 1 covers the basic worker lifecycle expected by users: create,
  start, observe status, inspect logs, execute follow-up commands, and clean up
  completed workloads.
- R7. Stage 1 must clearly reject project-backed tasks, per-job local config
  delivery, and other flows that still depend on host-mounted content.
- R8. Stage 1 should be framed as "Kubernetes runtime available" rather than as
  a complete replacement for Docker sandbox behavior.

**Stage 2: Project-Backed Common Tasks**
- R9. Stage 2 enables common project-backed tasks on Kubernetes by making the
  orchestrator the primary source of project content delivery.
- R10. Stage 2 must support a practical common-task set on delivered project
  content, including reading repository files, editing project files, running
  project-local commands, and returning changed files or artifacts through an
  explicit result-handoff flow rather than through silent live sync.
- R11. Stage 2 source-code changes must be handed back explicitly and only
  applied to the host project after user confirmation. The contract should
  support produced patches or changed-file sets, but should not imply automatic
  background write-back.
- R12. Stage 2 must make content freshness, snapshot provenance, and result
  handoff understandable to the user so that work performed in Kubernetes does
  not feel detached or surprising.
- R13. Stage 2 must keep large-project, high-churn, or otherwise unsupported
  project scenarios explicitly out of scope by default. When exceptions are
  needed, they should be allowed only through an explicit project-scoped admin
  override rather than an instance-wide or per-run bypass. Stage 2 should also
  extend the same orchestrator-delivery model to other per-job content that
  currently assumes local mounts, so common project-backed work is not blocked
  by sidecar configuration gaps.

**Stage 3: Near-Docker User Experience**
- R14. Stage 3 prioritizes user-facing parity over backend implementation
  parity: most users should be able to choose Kubernetes without learning a new
  mental model for normal work.
- R15. Stage 3 must make the Kubernetes path feel close to Docker in setup,
  capability discovery, task selection expectations, failure messaging, and
  recovery guidance.
- R16. Stage 3 must deliver Kubernetes-native controls that achieve a security
  and isolation outcome close enough to Docker that the product can describe
  both as first-class sandbox options, while still acknowledging any remaining
  edge-case differences. For allowlist-constrained networking in particular,
  the goal is not merely "some network restriction exists" but that common
  domain-bounded tasks feel close to the current Docker allowlist experience.
- R17. Stage 3 should make the common task-success profile close to Docker for
  typical day-to-day work, with backend-specific exceptions reduced to a small,
  well-documented set.

**Rollout and Product Communication**
- R18. Each stage must have an explicit feature status, supported-scenarios
  list, and non-goals list reflected in product docs and feature tracking.
- R19. Each stage must define observable exit criteria so planning and review
  can decide when the next stage is justified.
- R20. The staged roadmap must avoid broad promises about Kubernetes "sandbox
  parity" before the product can honestly support them.

## Success Criteria

- Stage 1 can be described truthfully as Kubernetes worker runtime support,
  without implying project-backed sandbox parity.
- Stage 2 unlocks a meaningful set of common project-backed tasks on Kubernetes
  without depending on host directory mounts as the primary path, and can hand
  source changes or artifacts back through an explicit confirm-before-apply
  flow.
- Stage 2 rejects oversized or otherwise unsupported repositories by default
  with clear guidance, while allowing narrowly scoped project-level exceptions
  when an administrator intentionally opts in.
- Stage 3 makes Kubernetes feel like a first-class option for most everyday
  work, including common allowlist-constrained networking tasks, with
  differences from Docker reduced to a small, explicit set.
- At every stage, users get predictable behavior and clear explanations of what
  is supported versus unsupported.

## Scope Boundaries

- This roadmap does not require Kubernetes to mimic Docker internals or use the
  same implementation strategy.
- This roadmap does not treat shared storage as the primary Stage 2 path.
- This roadmap does not require every Docker edge case to work on Kubernetes
  before Stage 3.
- This roadmap does not make `hostPath`-style local mounting a required part of
  the Kubernetes solution.
- This roadmap does not promise silent live synchronization between Kubernetes
  workspaces and the host project during Stage 2.
- This roadmap does not allow broad instance-wide or ad hoc per-run bypasses
  for large-repository safety limits as the default exception mechanism.
- This roadmap does not redefine the existing Docker path; it defines how the
  Kubernetes path matures beside it.

## Key Decisions

- **Platform-native first**: The goal is outcome parity for users, not
  implementation parity with Docker.
- **Orchestrator-delivered content is the Stage 2 foundation**: Project content
  should come from the orchestrator rather than local host mounts.
- **Stage 2 write-back is explicit, not silent**: Source changes should be
  returned to the host through an explicit handoff and only applied after user
  confirmation, rather than through background synchronization.
- **Large-repository safety is opt-in at the project level**: Oversized or
  high-churn repositories should be rejected by default, with explicit
  project-scoped administrative overrides for exceptional cases.
- **User experience parity is the Stage 3 priority**: The final stage is judged
  mainly by whether users can use Kubernetes without needing backend-specific
  knowledge for normal work.
- **Near-Docker networking means user-visible similarity, not just cluster-side
  policy presence**: Stage 3 should only claim parity when common
  allowlist-constrained tasks behave close to the existing Docker experience.
- **Staged honesty beats premature parity claims**: Product wording and feature
  status should stay aligned with the actual maturity level.

## Dependencies / Assumptions

- The orchestrator remains the trusted control point for worker lifecycle and
  content delivery.
- Planning will define the exact handoff format and apply rules for explicit
  change return in Stage 2, while preserving the product decision that host
  application requires user confirmation.
- Planning will define repository-size and churn thresholds for default
  rejection and how project-scoped overrides are represented and enforced.
- Security review will be required before Stage 3 can claim near-parity with
  Docker sandbox behavior.

## Outstanding Questions

### Deferred to Planning
- [Affects R10-R12][Technical] What exact change-return format should Stage 2
  use first: patch bundle, changed-file set, artifact bundle, or a hybrid?
- [Affects R11-R12][Technical] What are the host-side safety checks and failure
  rules for confirm-before-apply, especially when the underlying project moved
  since the snapshot was created?
- [Affects R13][Technical] What repository-size and churn thresholds should
  trigger default rejection, and which signals are cheap and reliable enough to
  enforce before job start?
- [Affects R13][Technical] Where should the project-scoped override live so it
  is explicit, reviewable, and hard to confuse with an instance-wide bypass?
- [Affects R16-R17][Needs research] Which Kubernetes-native network controls
  are sufficient for common allowlist-constrained tasks to feel close enough to
  the current Docker allowlist experience?
- [Affects R19][Technical] What measurable exit criteria best distinguish Stage
  2 incomplete, Stage 2 complete, and Stage 3 readiness for this roadmap?

## Next Steps

→ /prompts:ce-plan for structured implementation planning
