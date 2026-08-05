# APDD Governance Kit — Evaluation

**Status:** Evaluation (no code changes proposed in this branch — docs only)
**Date:** 2026-08-04
**Author:** Ron (with Claude Code)
**Source:** [`rdisandro/apdd-kit`](https://github.com/rdisandro/apdd-kit) @ `61daaa2` ("APDD Kit"), evaluated from a local clone.

> This document evaluates the **APDD Kit** ("Agent Product Design &
> Development") as a candidate governance framework for IronClaw. It describes
> what the kit is, how it is built, and — the load-bearing part — how it maps
> onto the governance IronClaw **already has**. The proposal and phased plan
> live in [PROPOSAL.md](PROPOSAL.md) and [INTEGRATION_PLAN.md](INTEGRATION_PLAN.md).

---

## 1. What the APDD Kit is

The APDD Kit is a **stack-agnostic, drop-in governance framework for building
UI/front-end software with an AI coding agent** (Claude Code, Cursor,
Antigravity, or any root-file-only tool). Its two stated priorities are
**minimize regression** and **raise product design/development quality**. It
ships as a set of copy-in artifacts — narrative guides, fill-in templates,
path-scoped agent rules, reusable skills, and enforcement (hooks + CI) — with
all product/vendor names left as `<angle-bracket>` placeholders.

Its defining design principle is that **an agent may encounter any single
artifact as the only governance in its context** — a root-file-only tool sees
just the instructions file; a path rule fires alone on an edit; a doc is read
cold. So each artifact is deliberately **self-sufficient and cross-redundant**,
and a deterministic **enforcement layer (hooks + CI) binds even when an agent
ignores the prose.** This is the same "assume non-compliance, make the machine
the backstop" philosophy IronClaw already applies in CI.

### The agentic operating system: four governance layers + a design track

The kit models governance as a small **operating system for agentic
development** — four layers that together turn "an agent editing files" into "an
agent operating inside guard-rails," plus a parallel design-governance pipeline:

| # | Layer | Mechanism | Question it answers |
|---|-------|-----------|---------------------|
| ① | **PLAN — docs-first feature workflow** | `docs/<slug>/{FEATURE_SPEC, IMPLEMENTATION_PLAN, TEST_PLAN}.md` + a central registry | *What is this feature supposed to do, and is my change in scope?* |
| ② | **GUIDE — auto-loading rules** | `.claude/rules/*.md` with `paths:` frontmatter, injected on matching edits | *What are the conventions/invariants for the file I'm touching right now?* |
| ③ | **PROTECT — Critical User Journeys** | `docs/_reference/CRITICAL_FLOWS.md` + a `critical-flows` hot-path rule | *Did my localized change break an end-to-end flow two layers away?* |
| ④ | **ENFORCE — hooks + CI** | pre-commit → pre-push → CI, self-skipping per check | *Does the machine block the mistake regardless of which tool wrote the code?* |

The **two standing rules** are the spine that ties the layers together:

- **Rule 1 — docs are the source of truth.** Read a feature's spec + plan
  before editing its code; if you add uncovered behavior, update the spec in the
  *same* change.
- **Rule 2 — critical bug fixes update the docs AND add a test, in the same
  diff.** Log it in the spec's *Critical Bug Fix Log*, revise the plan, and add
  a regression test that fails-before / passes-after.

Running alongside is a **Design/UX governance track** treated as *core*, not an
add-on (the kit is explicitly optimized for software with a UI):

1. **`DESIGN.md`** — a project-root "design constitution" (principles, theming,
   typography, a11y governance, component taxonomy, and a hard **REJECT list**),
   following Google's open DESIGN.md spec.
2. **Handoff flow** — a do-not-skip sequence from the design tool to production
   UI (context → metadata → screenshot → translate → validate 1:1).
3. **Design system / tokens / component taxonomy** — a 5-tier model (Tokens →
   Elements → Components → Patterns → Layouts) where **Tiers 2–3 stay pure and
   data-agnostic; only Tier 4 binds app state/services.**
4. **Storybook** — the component workbench, the agent's MCP interface
   (`get-documentation` so the agent **never hallucinates props**), and the test
   harness (play/interaction, token/CSS, a11y/axe, and visual/Chromatic tests —
   *stories as tests*).
5. **Rich-view performance** and **a validation gate + REJECT list** close it out.

### Opt-in modules

Four bundles cover capabilities only *some* UI products have: `realtime-media/`,
`rich-view/` (canvas/3D/GPU), `multitenant-isolation/`, and — most relevant to
IronClaw — **`embedded-ai-agents/`** (products that *ship* LLM agents, with
invariants for key encryption at rest, per-agent DB-backed config, and
tenant-scoped context on every LLM call).

### Portability

The kit ships in Claude Code conventions but explicitly maps each mechanism to
Cursor, Antigravity, and root-file-only tools. Hooks + CI are identical
everywhere because they are VCS/CI features, not agent features.

---

## 2. IronClaw's governance today (the honest baseline)

IronClaw is **not** a greenfield adopter. It has independently converged on a
large fraction of the kit's model. Concretely, in this repo today:

- **Path-scoped auto-loading rules already exist** — `.claude/rules/` holds 12
  rules (`architecture.md`, `cargo-features.md`, `database.md`,
  `error-handling.md`, `gateway-events.md`, `lifecycle.md`,
  `review-discipline.md`, `safety-and-sandbox.md`, `skills.md`, `testing.md`,
  `type-placement.md`, `types.md`). This *is* the kit's Layer ②.
- **Rule 2 is already enforced deterministically** — the commit-msg hook and
  [`.github/workflows/regression-test-check.yml`](../../../.github/workflows/regression-test-check.yml)
  require a regression test with every bug fix; [`.claude/rules/testing.md`](../../../.claude/rules/testing.md)
  encodes test-first + regression-with-every-fix; [CLAUDE.md](../../../CLAUDE.md)
  mandates red-then-green ordering. IronClaw's Rule-2 equivalent is arguably
  *stronger* than the kit's (CI-gated, not just prose).
- **A mature enforcement layer (Layer ④)** — ~30 workflows including
  `reborn-tests.yml`, `reborn-e2e.yml`, `reborn-playwright.yml`,
  `claude-review.yml`, `coverage.yml`, and the regression gate above.
- **A rich skills library** — `.claude/skills/` includes `reborn-feature`,
  `ironclaw-reborn-testing`, `ironclaw-reborn-architecture-review`,
  `ironclaw-reborn-orientation`, `reborn-extension-surfaces`, and more.
- **Strong product invariants already written down** — CLAUDE.md encodes
  "LLM data is never deleted," the `credential_name` / `extension_name` identity
  split, capability-dispatch boundaries, and safety/sandbox rules. These are
  exactly the class of invariant the kit's `embedded-ai-agents` module exists to
  pin.
- **A codebase knowledge graph + `openwiki/`** — a discovery layer the kit does
  not have and does not replace.

Where IronClaw diverges from the kit:

- **No structured, per-feature docs-first workflow (Layer ①).** Plans live in
  [`docs/plans/`](../) as flat, dated single files (`YYYY-MM-DD-<slug>.md`) —
  useful, but they are point-in-time plans, not *living* per-feature
  spec/plan/test sets with a registry and a code-to-feature map. There is no
  standing "read the spec before you edit this feature's code" trigger.
- **No Critical User Journey registry (Layer ③).** IronClaw has e2e/Playwright
  suites but no catalogued CUJ list with hot-path file mapping and an
  auto-loading `critical-flows` rule that fires when you touch a hot path.
- **No Design/UX governance track at all.** There is **no `DESIGN.md`**, no
  design-scoped `.claude/rules/`, no component taxonomy, no a11y gate, and no
  Storybook workbench — even though the frontend is a mature React SPA.

### The frontend is a near-perfect fit for the kit's design half

The WebUI frontend ([`crates/product/ironclaw_webui/frontend`](../../../crates/product/ironclaw_webui/frontend))
is **React 19 + Vite + Tailwind v4 + TypeScript + Vitest** — precisely the stack
the design track targets. It already has:

- an emerging **`src/design-system/`** (`theme.ts`, `theme.test.tsx`) and
  `src/styles/theme-colors.test.ts` — a design system *without a constitution*;
- **no Storybook configured** — no `.storybook/`, and Storybook is a declared
  dependency of **neither `package.json` nor `pnpm-lock.yaml`**. (A stray
  `storybook`/`@storybook` copy exists in the gitignored local `node_modules`,
  but it is not lockfile-backed — a clean `pnpm install` would not restore it,
  so it signals nothing about intent);
- a real component/page tree (`src/components/`, `src/pages/{chat,missions,
  projects,settings,extensions,onboarding,…}`), and light/dark theming.

In other words, the single biggest gap the kit fills (design governance) lands
on a frontend that is a small, well-understood step from adopting it — a pinned
`pnpm add -D` of Storybook + addons plus a `.storybook/` config (Phase 2 of the
plan) — on a stack (React 19 / Vite / Tailwind v4 / Vitest, pnpm@11.7.0) that
Storybook supports directly.

---

## 3. Overlap / gap matrix

Legend: **✅ Have** (IronClaw already does this) · **◑ Partial** (exists but
informal / not wired as the kit does) · **❌ Gap** (absent).

| Kit component | Layer | IronClaw today | Verdict |
|---|---|---|---|
| Path-scoped `.claude/rules/*.md` (`paths:` auto-load) | ② | 12 rules in `.claude/rules/` | **✅ Have** — same mechanism |
| Rule 2: regression test + doc update per critical fix | ①/④ | commit-msg hook + `regression-test-check.yml` + `testing.md` | **✅ Have** — CI-enforced, stronger than kit |
| Enforcement: hooks + CI, self-skipping, aggregate gate | ④ | ~30 workflows, path-filtered jobs | **✅ Have** |
| Reusable, user-invocable skills | — | `.claude/skills/` (reborn-feature, testing, review…) | **✅ Have** |
| Test-first discipline | ①/④ | CLAUDE.md + testing.md mandate red-then-green | **✅ Have** |
| Embedded-AI-agent invariants (keys at rest, scoped context) | module | Encoded across CLAUDE.md / safety rules, not one list | **◑ Partial** — captured but scattered |
| Rule 1: docs-are-source-of-truth, read-spec-before-edit | ① | `reborn-feature` skill + `docs/plans/*` (ad-hoc, flat) | **◑ Partial** — no living per-feature set / registry |
| `docs/<slug>/{SPEC,PLAN,TEST}` + FEATURE_REGISTRY | ① | dated single-file plans only | **❌ Gap** (structure) |
| Code-to-feature mapping (`// Feature: <slug>` headers) | ① | knowledge graph + openwiki instead | **◑ Partial** — different, backend-oriented |
| Critical User Journeys registry + `critical-flows` rule | ③ | e2e/Playwright exist; no CUJ catalog/rule | **❌ Gap** |
| `DESIGN.md` design constitution | design | none | **❌ Gap** |
| Design/UX auto-loading rules (styling, a11y, taxonomy) | design | none | **❌ Gap** |
| Component taxonomy (5-tier purity model) | design | implicit in `components/` vs `pages/` | **◑ Partial** — not codified |
| Design tokens governance (no raw hex, light+dark) | design | `design-system/theme.ts` exists, ungoverned | **◑ Partial** |
| Storybook workbench + MCP + stories-as-tests | design | in `node_modules`, **unconfigured** | **❌ Gap** (high-value, low-friction) |
| Accessibility gate (contrast, focus, semantics) | design | none formal | **❌ Gap** |
| Codebase knowledge graph + `openwiki/` | — | present | **Kit lacks this** — IronClaw is ahead |

**Reading of the matrix:** IronClaw has essentially *finished* the kit's
backend/enforcement half (Layers ② and ④, plus Rule 2 and skills). The kit's net
new value for IronClaw is concentrated in **Layer ① (a living docs-first
workflow), Layer ③ (a CUJ regression baseline), and the entire Design/UX
track** — the last of which IronClaw lacks completely and which fits its
frontend with unusually low friction.

---

## 4. Strengths, risks, and honest caveats

### Strengths (why this is worth adopting selectively)

- **It validates IronClaw's existing direction.** The kit is independent
  evidence that the `.claude/rules/` + CI-enforced-regression model IronClaw
  already runs is a sound, generalizable pattern — not a local quirk.
- **The design track is genuinely missing and genuinely valuable.** Agentic UI
  work without a `DESIGN.md` and a Storybook MCP is exactly where AI agents
  hallucinate props, invent hex values, and skip a11y. This is the kit's
  highest-leverage contribution for IronClaw.
- **Enforcement-first philosophy matches IronClaw's.** "Assume non-compliance;
  make the machine bind it" is already how IronClaw runs CI, so the cultural
  fit is high.
- **Everything is copy-in and reversible.** Adoption is additive; nothing forces
  a rewrite.

### Risks and caveats

- **Layer-model mismatch on the backend.** The kit's example architecture is
  **MVVM + Service Layer** for a client app (Views/ViewModels/Services/Models).
  IronClaw's backend is a Rust **Reborn crate stack** with its own architecture
  rules and a knowledge-graph discovery model. The kit's *backend* layer rules
  (`views.md`, `viewmodels.md`, `services.md`, `models.md`,
  `server-architecture.md`) **do not map to IronClaw's crates** and should be
  **skipped** — IronClaw's `.claude/rules/architecture.md` already owns that
  boundary. Adopting them would be redundant at best, contradictory at worst.
- **Cross-boundary redundancy is deliberate — but IronClaw favors single-owner
  docs.** The kit intentionally overlaps prose across artifacts. IronClaw's
  culture (CLAUDE.md, single-owner contracts, "dedupe within a boundary") is
  compatible but requires discipline to avoid drift. Adopt the redundancy model
  *only* where a rule can fire in isolation.
- **Docs-first can bureaucratize if applied to the backend wholesale.** IronClaw
  already ships features via the `reborn-feature` skill + integration tests.
  Forcing a three-doc set on every backend crate change would add ceremony
  without payoff. The docs-first workflow should be **scoped to the WebUI /
  product-surface features** where design + UX + behavior genuinely benefit from
  a spec, and stay lightweight elsewhere.
- **Storybook + Chromatic add CI cost and maintenance.** Visual regression is
  powerful but not free (snapshot baselines, a Chromatic account, CI minutes).
  Start with the local/free subset (play + a11y + token tests in Vitest) and add
  Chromatic only if the value shows.
- **Two "Rule 1 / Rule 2" numbering schemes could collide** with IronClaw's own
  rule vocabulary. Namespacing matters (see the plan).

### Bottom line

Adopt the kit **as a design-and-workflow overlay, not a wholesale replacement**:
take the Design/UX track (highest value), the docs-first feature workflow
(scoped to product/WebUI features), and the CUJ regression baseline; **skip the
backend MVVM layer rules** because IronClaw's crate architecture rules already
own that ground; and treat the `embedded-ai-agents` module as a **checklist to
consolidate invariants IronClaw has already written but scattered.**

Continue to [PROPOSAL.md](PROPOSAL.md) for the recommendation and per-component
scope, and [INTEGRATION_PLAN.md](INTEGRATION_PLAN.md) for the phased rollout.
