# APDD Governance Kit — Evaluation & Integration Proposal

Evaluation of the **APDD Kit** ("Agent Product Design & Development") — a
stack-agnostic governance framework for building UI software with an AI coding
agent — and a proposal + phased plan for integrating it into IronClaw.

**Source evaluated:** [`rdisandro/apdd-kit`](https://github.com/rdisandro/apdd-kit)
@ `61daaa2` (evaluated from a local clone). The kit repository is **private**;
[EVALUATION.md](EVALUATION.md) §1 is the self-contained description of what was
evaluated for reviewers without access.

## TL;DR — the decision

> **Adopt selectively as a design-and-workflow overlay, not a replacement.**
> IronClaw has already independently built the kit's **backend/enforcement
> half** — path-scoped `.claude/rules/`, CI-enforced regression tests, a rich
> skills library, ~30 CI workflows. The kit's **net-new value** is its
> **Design/UX governance track** (which IronClaw lacks entirely) and a **living
> docs-first feature workflow** — and both land on a React/Vite/Tailwind/Vitest
> frontend (with an emerging `src/design-system/`; Storybook is not yet a
> declared dependency, so adoption is a small, pinned install) that fits them
> with unusually low friction. **Skip** the kit's
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
agentic-OS framework — plus the IronClaw overlap/gap map and rollout — lives in
[explorer.html](explorer.html): a **self-contained** page (no build step; open
in any browser, or render it without cloning via
[html-preview](https://html-preview.github.io/?url=https://github.com/nearai/ironclaw/blob/main/docs/internal/apdd-governance-kit/explorer.html) —
that link resolves once this lands on `main`; before then, open the file from
the PR branch).
It is a convenience view; the Markdown in this folder is the source of truth.

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
| `explorer.html` | Self-contained diagram-first review page (open in any browser) |
