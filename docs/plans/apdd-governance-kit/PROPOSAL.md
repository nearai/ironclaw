# APDD Governance Kit — Proposal

**Status:** Proposal for review
**Date:** 2026-08-04
**Depends on:** [EVALUATION.md](EVALUATION.md)

---

## 1. Recommendation

**Adopt the APDD Kit selectively, as a design-and-workflow overlay on
IronClaw's existing governance — not as a replacement.** Concretely:

> **Take** the Design/UX governance track (DESIGN.md + design rules + Storybook
> workbench), the docs-first feature workflow **scoped to WebUI/product-surface
> features**, and the Critical User Journey regression baseline.
> **Adapt** the `embedded-ai-agents` module into a consolidated invariants
> checklist. **Skip** the kit's backend MVVM layer rules — IronClaw's Rust crate
> architecture rules already own that boundary.

The reasoning, in one line: **IronClaw has already built the kit's
backend/enforcement half; the kit's net-new value is its design half and its
living feature-workflow — and both land on a frontend that fits them with
unusually low friction.**

This is a **medium-effort, high-value, fully reversible** adoption. Every piece
is additive and copy-in; nothing forces a rewrite of existing crates or CI.

---

## 2. Guiding principles for the integration

1. **Additive, never disruptive.** Layer the kit *around* existing governance.
   Where IronClaw already has a mechanism (path rules, regression CI, skills),
   keep IronClaw's — do not import a second copy.
2. **Single owner per boundary.** IronClaw favors single-owner contracts. Adopt
   the kit's deliberate cross-boundary redundancy *only* where a rule genuinely
   fires in isolation (a design rule on a cold UI edit); otherwise keep one
   authoritative home.
3. **Design track is the headline.** The clearest, most defensible win is design
   governance on the WebUI. Lead with it.
4. **Scope docs-first to where a spec pays for itself.** Product/WebUI features
   with real UX and multi-PR scope — yes. A one-file crate refactor — no; the
   existing exemptions and the `reborn-feature` skill already cover it.
5. **Namespace to avoid collisions.** The kit's "Rule 1 / Rule 2" and its layer
   rule filenames must not shadow IronClaw's existing rule vocabulary. Prefix
   design rules (`design-*.md`) and reference the kit's rules as "the feature
   workflow" rather than re-numbering IronClaw's rules.

---

## 3. Per-component decision: Adopt / Adapt / Skip

### ✅ ADOPT (bring in largely as-is, with IronClaw substitutions)

| Component | Why | IronClaw target |
|---|---|---|
| **`DESIGN.md` constitution** | Highest-leverage gap; agentic UI without it invites hallucinated props/hex and skipped a11y. | New root or `docs/design/DESIGN.md`, seeded from `templates/DESIGN.template.md`; substitute React/Tailwind/`ironclaw` token prefix. |
| **Design/UX `.claude/rules/`** (styling, theming, a11y, taxonomy, handoff) | Auto-load on frontend edits — the mechanism IronClaw already trusts, applied to a surface that has none. | `.claude/rules/design.md` (+ optional `design-a11y.md`), `paths:` scoped to `crates/product/ironclaw_webui/frontend/src/**`. |
| **Storybook workbench + MCP + stories-as-tests** | Isolated component dev, `get-documentation` prop-grounding, and a11y/token/interaction tests in the existing Vitest. Storybook is *not* a declared dependency today (absent from `package.json` and `pnpm-lock.yaml`), so adoption is an explicit, pinned install — not "already there." | Add pinned `storybook`, the React/Vite framework adapter `@storybook/react-vite`, and `@storybook/addon-{vitest,a11y,docs,mcp}` as `devDependencies` via **pnpm** (set `framework: '@storybook/react-vite'` in `.storybook/main.ts`), commit the lockfile, configure `.storybook/`, and author stories per Tier-2/3 component. |
| **Component taxonomy (5-tier purity model)** | Codifies the implicit `components/` (pure) vs `pages/` (bound) split so Tiers 2–3 stay renderable in isolation. | Section in `DESIGN.md`; enforced by the design rule + REJECT list. |
| **Critical User Journeys registry + `critical-flows` rule** | Turns IronClaw's e2e/Playwright coverage into a named regression baseline with a hot-path auto-load trigger. | `docs/qa/CRITICAL_FLOWS.md` + `.claude/rules/critical-flows.md`. |

### ◑ ADAPT (take the idea; fit it to IronClaw's shape)

| Component | Adaptation |
|---|---|
| **Docs-first feature workflow (Layer ①)** | Adopt the three-doc set + registry, but **scope it to WebUI/product-surface features** and prefer the **lightweight single-file `FEATURE.md` tier** for most. Keep the existing `docs/plans/*` dated plans for backend/infra epics. Use a light `docs/<slug>/` under a product-features area rather than mandating it repo-wide. |
| **`embedded-ai-agents` module** | IronClaw *is* an embedded-AI-agent product. Consolidate the module's invariants into one referenced "What NOT to do" list rather than importing its multi-tenant SaaS framing. Map **each** invariant named in [EVALUATION.md](EVALUATION.md) §1 to its IronClaw home so none is lost: **(a) keys encrypted at rest** → the secrets/credential-storage path (the `credential_name` identity + safety rules); **(b) per-agent config in the DB, not env vars** → IronClaw's DB-backed config persisted through the `RootFilesystem` mount catalog; **(c) tenant/scoped context on every LLM call** → capability-dispatch scoping + the scoped-filesystem tenant isolation; plus IronClaw's own **LLM-data-is-never-deleted** and **credential/extension identity** invariants. Each already exists in a CLAUDE.md/safety rule — the consolidation only cross-links them. |
| **Code-to-feature mapping (`// Feature: <slug>`)** | For the **frontend**, adopt lightweight headers so the design/feature docs are discoverable on edit. For the **backend**, keep the knowledge graph + `openwiki/` — it is a superior discovery layer the kit lacks. Do not blanket the crates with comment headers. |
| **Enforcement templates** | IronClaw's CI already exceeds the kit's. **Adapt only the design-specific additions**: a Storybook/a11y CI job and (optionally later) Chromatic visual regression. Do not touch the existing hook/CI stack. |

### ❌ SKIP (redundant or mismatched)

| Component | Why skip |
|---|---|
| **Backend MVVM layer rules** (`views.md`, `viewmodels.md`, `services.md`, `models.md`, `server-architecture.md`) | The kit's client-MVVM model does not map to IronClaw's Rust Reborn crate stack. `.claude/rules/architecture.md` + the architecture-review skill already own this boundary. Importing these would duplicate or contradict existing rules. |
| **Kit `database.md`, `testing.md`, `performance.md` (backend)** | IronClaw already has richer, repo-specific versions of all three in `.claude/rules/`. |
| **Kit enforcement hooks / CI templates (wholesale)** | IronClaw's ~30-workflow CI + commit-msg regression gate is more mature. Only the design-CI additions are new. |
| **`realtime-media/`, `rich-view/`, `multitenant-isolation/` modules** | Not applicable to IronClaw's current product shape. (Revisit `rich-view` only if a canvas/GPU surface ships.) |
| **Project-generator / signing conventions** (XcodeGen-style) | IronClaw uses Cargo + pnpm; no generated IDE project. |

---

## 4. What "done" looks like (success criteria)

- A **`DESIGN.md`** exists and is referenced from CLAUDE.md as a hard gate for
  UI work; its REJECT list is concrete and testable.
- Editing any `crates/product/ironclaw_webui/frontend/src/**` file **auto-loads a design
  rule** (tokens, a11y, taxonomy, handoff) — verified by inspection.
- **Storybook runs** in the frontend with the MCP addon, and at least the
  **atoms/primitives** ship stories with a smoke play test, a token/CSS check,
  and **passing a11y (axe) checks for every story in the covered set** — which
  Phase 2 defines as *all* its primitive stories, so Phase 2 exits only when
  every one of them is at `test: 'error'` with zero violations (no covered story
  left at `'todo'`). These run in a Storybook browser-mode Vitest project; in
  CI, the gate runs the full covered set at `test: 'error'` (a violation fails
  the job), `'todo'` is reserved for stories added by later work, and
  changed-story filtering is a local fast path, not the gate (see the Phase 2
  a11y-gate definition in
  [INTEGRATION_PLAN.md](INTEGRATION_PLAN.md)).
- A **`docs/qa/CRITICAL_FLOWS.md`** catalogs IronClaw's real end-to-end journeys (e.g.
  onboarding/pairing, chat turn, extension auth, mission run) with hot-path
  files, and a `critical-flows` rule auto-loads on those paths.
- The **embedded-AI-agent invariants are consolidated** into one referenced
  list; no invariant is lost, none newly contradicted.
- **Zero regressions** to existing CI, hooks, crate rules, or the knowledge
  graph. Backend workflow is unchanged.

---

## 5. Non-goals

- **Not** replacing IronClaw's `.claude/rules/`, CI, or the knowledge-graph /
  `openwiki/` discovery layer.
- **Not** mandating the three-doc workflow for backend crate changes.
- **Not** adopting the kit's MVVM/client-architecture model anywhere in the
  Rust stack.
- **Not** standing up Chromatic/paid visual regression in the first phase.

See [INTEGRATION_PLAN.md](INTEGRATION_PLAN.md) for sequencing, effort, and risks.
