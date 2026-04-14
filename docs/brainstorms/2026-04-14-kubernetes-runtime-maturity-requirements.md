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
  project-local commands, and returning changed files or artifacts through the
  normal product flow.
- R11. Stage 2 must make content freshness and task scope understandable to the
  user so that work performed in Kubernetes does not feel detached or
  surprising.
- R12. Stage 2 must keep large-project, high-churn, or otherwise unsupported
  project scenarios explicitly out of scope until they have a defined contract.
- R13. Stage 2 should extend the same orchestrator-delivery model to other
  per-job content that currently assumes local mounts, so common project-backed
  work is not blocked by sidecar configuration gaps.

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
  edge-case differences.
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
  without depending on host directory mounts as the primary path.
- Stage 3 makes Kubernetes feel like a first-class option for most everyday
  work, with differences from Docker reduced to a small, explicit set.
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
- This roadmap does not redefine the existing Docker path; it defines how the
  Kubernetes path matures beside it.

## Key Decisions

- **Platform-native first**: The goal is outcome parity for users, not
  implementation parity with Docker.
- **Orchestrator-delivered content is the Stage 2 foundation**: Project content
  should come from the orchestrator rather than local host mounts.
- **User experience parity is the Stage 3 priority**: The final stage is judged
  mainly by whether users can use Kubernetes without needing backend-specific
  knowledge for normal work.
- **Staged honesty beats premature parity claims**: Product wording and feature
  status should stay aligned with the actual maturity level.

## Dependencies / Assumptions

- The orchestrator remains the trusted control point for worker lifecycle and
  content delivery.
- Planning will define a clear "common task" envelope for Stage 2 that is large
  enough to matter but narrow enough to keep the contract honest.
- Security review will be required before Stage 3 can claim near-parity with
  Docker sandbox behavior.

## Outstanding Questions

### Deferred to Planning
- [Affects R10][Technical] What is the initial "common task" envelope for Stage
  2, and which task classes should be deferred even after orchestrator-based
  content delivery exists?
- [Affects R11][Technical] How should the product communicate freshness,
  synchronization, and result handoff so Kubernetes-backed work feels coherent
  to users?
- [Affects R16][Needs research] Which Kubernetes-native controls are sufficient
  for the product to honestly describe Stage 3 as near-parity with Docker from
  a user perspective?
- [Affects R19][Technical] What measurable exit criteria best distinguish Stage
  1 completion from Stage 2 readiness, and Stage 2 completion from Stage 3
  readiness?

## Next Steps

→ /prompts:ce-plan for structured implementation planning
