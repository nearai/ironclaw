# APDD Governance Kit — Evaluation & Integration Proposal

Evaluation of the **APDD Kit** ("Agent Product Design & Development") — a
stack-agnostic governance framework for building UI software with an AI coding
agent — and a proposal + phased plan for integrating it into IronClaw.

**Source evaluated:** `/Users/rondisandro/devprojects/github/apdd-kit` (git
`61daaa2`).

## TL;DR — the decision

> **Adopt selectively as a design-and-workflow overlay, not a replacement.**
> IronClaw has already independently built the kit's **backend/enforcement
> half** — path-scoped `.claude/rules/`, CI-enforced regression tests, a rich
> skills library, ~30 CI workflows. The kit's **net-new value** is its
> **Design/UX governance track** (which IronClaw lacks entirely) and a **living
> docs-first feature workflow** — and both land on a React/Vite/Tailwind/Vitest
> frontend (with an emerging `src/design-system/` and Storybook already in
> `node_modules`) that fits them with unusually low friction. **Skip** the kit's
> backend MVVM layer rules — IronClaw's crate architecture rules own that ground.

**Effort:** ~1–2 weeks for the full rollout; **~1 week (Phases 0–2) captures the
majority of the value.** Fully additive and reversible.

## Reading order

1. **[EVALUATION.md](EVALUATION.md)** — what the kit is (the four-layer agentic
   OS + design track), IronClaw's honest baseline, and the overlap/gap matrix.
2. **[PROPOSAL.md](PROPOSAL.md)** — the recommendation and the per-component
   **Adopt / Adapt / Skip** decisions.
3. **[INTEGRATION_PLAN.md](INTEGRATION_PLAN.md)** — the phased rollout with file
   targets, effort, risks, and open questions.

## The human-review artifact

A visual, diagram-first overview of the kit's benefits, functionality, and the
agentic-OS framework — plus the IronClaw overlap/gap map and rollout — is
published as a shareable claude.ai artifact. See the link in the conversation /
PR description that accompanies this branch.

## What this branch contains

Docs only — **no code, rule, or CI changes.** This branch is the evaluation and
proposal; the phased plan above is what a *follow-on* implementation branch would
execute, gated by review.

| File | Purpose |
|---|---|
| `README.md` | This index + the decision |
| `EVALUATION.md` | Kit anatomy + IronClaw overlap/gap analysis |
| `PROPOSAL.md` | Recommendation + Adopt/Adapt/Skip scope |
| `INTEGRATION_PLAN.md` | Phased rollout, effort, risks, open questions |
