# APDD Governance Kit — Integration Plan

**Status:** Proposed plan (pending review)
**Date:** 2026-08-04
**Depends on:** [EVALUATION.md](EVALUATION.md), [PROPOSAL.md](PROPOSAL.md)

A phased, reversible rollout. Each phase is independently shippable and gated by
review. Effort is a rough order of magnitude for one engineer + agent.
Phases 0–2 deliver most of the value; Phases 3–4 are follow-on.

---

## Phase 0 — Foundations & decisions (½ day)

**Goal:** lock the small set of naming/placement decisions so nothing has to be
renamed later.

| Step | Action | Target |
|---|---|---|
| 0.1 | Decide **`DESIGN.md` location**: repo root vs `docs/design/DESIGN.md`. Recommendation: `docs/design/DESIGN.md` (keeps root clean; `docs/design/` already exists) with a one-line pointer from CLAUDE.md. | — |
| 0.2 | Decide **feature-docs home** for WebUI features: recommend `docs/features/<slug>/` (distinct from the dated `docs/plans/`), or the lightweight `FEATURE.md` tier. | — |
| 0.3 | Decide **token prefix** and confirm the design-system substitutions (React, Tailwind v4 CSS-config, `ironclaw` prefix). | `src/design-system/` |
| 0.4 | Confirm **skip list** (backend MVVM rules, kit CI/hooks, N/A modules) with reviewers. | — |

**Exit:** a short decisions note (can live in this folder's README) that Phases
1–4 reference. No code yet.

---

## Phase 1 — Design constitution + design rules (1–2 days) — **highest value**

**Goal:** give the WebUI a governing `DESIGN.md` and make it auto-load on every
frontend edit. This is the single biggest gap and the clearest win.

| Step | Action | Target |
|---|---|---|
| 1.1 | Seed `DESIGN.md` from `apdd-kit/templates/DESIGN.template.md`; fill §1 principles, §2 theming (map to `src/design-system/theme.ts` light/dark), §3 typography, §4 a11y governance, §6 taxonomy tiers, **§7 REJECT list**. | `docs/design/DESIGN.md` |
| 1.2 | Write `.claude/rules/design.md` (adapted from `apdd-kit/rules/design.md`): `paths:` → `crates/product/ironclaw_webui/frontend/src/**`; content = styling/theming/a11y/taxonomy + "read DESIGN.md first" + REJECT gate. | `.claude/rules/design.md` |
| 1.3 | Codify the **token rule**: no raw hex at call sites; every custom color ships light+dark; `ironclaw`-prefixed semantic tokens. Reconcile with the existing `theme-colors.test.ts`. | `DESIGN.md` §2 + rule |
| 1.4 | Add a CLAUDE.md pointer: "UI work conforms to `docs/design/DESIGN.md`; its REJECT list is a hard gate." | `CLAUDE.md` |
| 1.5 | Consolidate the **embedded-AI-agent invariants** into a single referenced "What NOT to do" list, mapping **every** invariant named in EVALUATION §1 to its IronClaw home so none is lost: **keys encrypted at rest** (secrets/credential-storage path), **per-agent config in the DB not env vars** (`RootFilesystem`-persisted config), **tenant/scoped context on every LLM call** (capability-dispatch + scoped-filesystem isolation), **LLM-data-never-deleted**, and **credential/extension identity** — cross-linking existing CLAUDE.md/safety rules, not duplicating them. | `CLAUDE.md` / `.claude/rules/` |

**Exit:** editing a frontend file surfaces the design rule; `DESIGN.md` is
concrete enough to reject a raw-hex or unlabeled-control diff. **No runtime code
changes** — pure governance.

**Risk:** over-specifying `DESIGN.md` before the design system is mature →
mitigate by keeping §§ that IronClaw can honor today and marking aspirational
items `[PENDING]`.

---

## Phase 2 — Storybook workbench + stories-as-tests (2–4 days)

**Goal:** isolated component development, prop-grounding via MCP (agent never
hallucinates props), and a11y/token/interaction tests in the existing Vitest.

| Step | Action | Target |
|---|---|---|
| 2.1 | Add Storybook via **pnpm** (`pnpm@11.7.0`, the repo's declared package manager) as **pinned `devDependencies`** — `storybook`, the **React/Vite framework adapter `@storybook/react-vite`**, and the addons in 2.2 — commit the updated `pnpm-lock.yaml`, then run init/config. Storybook is a declared dependency of **neither `package.json` nor the lockfile** today, so treat this as a fresh, pinned install and **verify from a clean `pnpm install --frozen-lockfile`** — do not rely on any stray copy in the local (gitignored) `node_modules`. | `crates/product/ironclaw_webui/frontend` |
| 2.2 | Configure `.storybook/main.ts`: set **`framework: '@storybook/react-vite'`** (required for a React/Vite project) and register addons `@storybook/addon-vitest`, `-a11y`, `-docs`, `-mcp` (+ `@chromatic-com/storybook` deferred to Phase 4). | `.storybook/` |
| 2.3 | `.storybook/preview.ts`: import the global stylesheet (tokens load) and wrap stories in the app theme provider decorator. **a11y mode:** covered stories run at **`test: 'error'`** (fail on any violation); only not-yet-triaged stories sit at `'todo'` — see the CI gate in 2.6. | `.storybook/preview.ts` |
| 2.4 | Author stories for **Tier-2 primitives first** (buttons, inputs, the items in `src/components/`): a smoke play test, a **token/CSS check** (computed style == token), and variant stories per visual state. | `src/**/*.stories.tsx` |
| 2.5 | Wire the **Storybook MCP** (`http://localhost:6006/mcp`) and add a CLAUDE.md / design-rule line: "query Storybook MCP `get-documentation` before using any component — never guess props." | `CLAUDE.md` + rule |
| 2.6 | Add the **Storybook test job to `reborn-tests.yml`** (the existing WebUI frontend lane). That lane today runs `vitest run` (VM/happy-dom) + `pnpm build` — **not browser-based** — so the Storybook Vitest addon needs its **own browser-mode Vitest project** (Playwright Chromium), separate from `vitest run`. **CI-observable a11y gate:** `test: 'todo'` prints nothing in CI and so cannot gate; instead run **covered stories at `test: 'error'`** (a violation fails the job) and leave only untriaged stories at `'todo'`, and emit the axe results as a job artifact for visibility. This `error`-on-covered job *is* the "passing a11y (axe) checks" success criterion in PROPOSAL.md. **Coverage contract:** the CI gate runs the **full covered-story set** every run (that is what backs the "every covered story passes" guarantee); changed-story filtering is an optional **local / pre-push fast path only**, never the gate. | `.github/workflows/reborn-tests.yml` |

**Exit:** `pnpm storybook` works **from a `--frozen-lockfile` install**;
primitives have stories that run in the **browser-mode Vitest project**; **every
covered story passes axe at `test: 'error'`** in CI (untriaged stories stay
`'todo'`); the agent can introspect real props via MCP.

**Risks:** (a) Storybook version drift vs. React 19 / Tailwind v4 — pin every
Storybook package (incl. `@storybook/react-vite`) and smoke-test the build.
(b) CI minutes — the covered set starts small (primitives first), so the
full-set gate is cheap; the main cost is the one-time Playwright Chromium
install, which should be cached. (Changed-story filtering stays a local fast
path, not a way to shrink the gate.) (c) Tailwind v4 CSS-config (no `tailwind.config.js`)
— ensure the preview imports the same CSS entry the app uses so tokens resolve.

---

## Phase 3 — Docs-first feature workflow (scoped) + registry (1–2 days)

**Goal:** a *living* per-feature doc set for WebUI/product-surface features, with
Rule 1 discoverability — without bureaucratizing backend work.

| Step | Action | Target |
|---|---|---|
| 3.1 | Create `docs/features/_templates/` from the kit's `FEATURE_SPEC` / `IMPLEMENTATION_PLAN` / `TEST_PLAN` / `FEATURE_LITE` / `FEATURE_REGISTRY` templates (substitute IronClaw terms). | `docs/features/_templates/` |
| 3.2 | Pilot with **one real WebUI feature** — e.g. adopt an existing surface (onboarding, chat, or the agent-activity-streaming work) into a `docs/features/<slug>/FEATURE.md` (lightweight tier) to prove the flow end-to-end. | `docs/features/<slug>/` |
| 3.3 | Add a **`feature-workflow` design-scoped rule** (adapted from `apdd-kit/rules/feature-workflow.md`) with `paths:` limited to `crates/product/ironclaw_webui/frontend/src/**` (+ product-surface crates if desired) — so Rule 1 fires only where a spec pays for itself. | `.claude/rules/` |
| 3.4 | Add lightweight `// Feature: <slug>` headers to **frontend** feature files only; keep backend discovery on the knowledge graph + `openwiki/`. | frontend `src/**` |
| 3.5 | Seed the registry; document that the feature-doc header `Status` is canonical and the registry mirrors it. | `docs/features/_templates/README.md` |

**Exit:** one feature fully governed by the workflow; the rule auto-loads on
frontend edits; clear scoping note that backend uses the existing model.

**Risk:** scope creep into backend → the `paths:` glob and the PROPOSAL non-goals
are the guard-rail; review must hold the line.

---

## Phase 4 — Critical User Journeys + visual regression (2–3 days)

**Goal:** name IronClaw's must-never-break flows and gate them; add visual
regression if Phase 2 proved its worth.

| Step | Action | Target |
|---|---|---|
| 4.1 | Catalog real CUJs in `CRITICAL_FLOWS.md`: onboarding/pairing, a chat turn end-to-end, extension auth (Slack/Telegram/Gmail), mission/routine run, notification delivery — each with hot-path files + a smoke checklist. Cross-reference existing e2e/Playwright coverage. | `docs/qa/CRITICAL_FLOWS.md` |
| 4.2 | Add `.claude/rules/critical-flows.md` with `paths:` = the hot-path files, requiring the matching CUJ be run/considered before "done." | `.claude/rules/` |
| 4.3 | Map each CUJ to its existing automated coverage (`reborn-e2e`, `reborn-playwright`) and flag any CUJ with no automation as a coverage gap. | `CRITICAL_FLOWS.md` |
| 4.4 | *(Optional)* Add **Chromatic** visual regression for Storybook stories if Phase 2 adoption is healthy and the CI budget allows. | `.storybook/` + CI |

**Exit:** a named regression baseline tied to real automation; a hot-path edit
auto-loads the CUJ rule.

**Risk:** CUJ list drifts from reality → keep it short (5–8 flows) and reviewed
with each release, exactly as the kit prescribes.

---

## Sequencing & dependencies

```
Phase 0 ──► Phase 1 (design constitution)          ← highest value, ship first
                │
                ├─► Phase 2 (Storybook) ─────────────────────┐
                │      depends on §1 tokens/taxonomy          │  needed by 4.4
                │                                             ▼  (visual regression)
                └─► Phase 3 (docs-first) ───────────► Phase 4 (CUJs + visual regression)
                       parallel with Phase 2
```

**Phase 4 has two inbound dependencies:** the CUJ work builds on the Phase 3
feature workflow, *and* its optional visual-regression step (4.4, Chromatic)
builds on the Phase 2 Storybook setup. If Phase 2 is deferred, Phase 4 still
ships its CUJ half; only 4.4 waits.

Phases 1 and 3 are pure-governance (docs + rules) and carry near-zero runtime
risk. Phases 2 and 4 touch the frontend toolchain/CI and warrant the most
testing.

## Effort summary

| Phase | Effort | Value | Runtime risk |
|---|---|---|---|
| 0 — Foundations | ½ day | enabling | none |
| 1 — DESIGN.md + design rules | 1–2 days | **high** | none |
| 2 — Storybook + stories-as-tests | 2–4 days | **high** | low–med (toolchain/CI) |
| 3 — Docs-first (scoped) | 1–2 days | medium | none |
| 4 — CUJs + visual regression | 2–3 days | medium–high | low–med |

**Total:** ~1–2 weeks of focused work for the full rollout; **Phases 0–2 (~1
week) capture the majority of the value.**

## Rollback

Every artifact is additive; no existing crate, rule, hook, or workflow is
modified destructively in any phase. A full rollback has two parts — **delete
the added files** *and* **revert the edits to existing files** — because
deleting only the added docs would leave stale pointers and active rules behind.

**Added files to delete** — delete **only the exact files this rollout added**,
never a blanket glob (each phase records the files it introduces; do not delete
feature docs or stories authored by other work):
- `docs/design/DESIGN.md`
- `.claude/rules/design.md` (and `design-a11y.md` if created), `.claude/rules/feature-workflow.md`, `.claude/rules/critical-flows.md`
- `docs/qa/CRITICAL_FLOWS.md`
- the `docs/features/_templates/` scaffold and the specific piloted `docs/features/<slug>/` folder(s) this rollout created — **not** a blanket `docs/features/` delete
- `crates/product/ironclaw_webui/frontend/.storybook/` and the specific `*.stories.tsx` files this rollout added — **not** every `*.stories.tsx`

**Edits to existing files to revert with targeted patches** (revert only the
rollout's hunks — do not wholesale-revert a file that other work also touched):
- `CLAUDE.md` — the DESIGN.md pointer, the Storybook-MCP line, and the consolidated invariant list (Phase 1.4/1.5, 2.5)
- `crates/product/ironclaw_webui/frontend/package.json` + `pnpm-lock.yaml` — the pinned Storybook `devDependencies`
- `.github/workflows/reborn-tests.yml` — the Storybook/a11y test job

**Compatibility / hidden side effects:** reverting `package.json` +
`pnpm-lock.yaml` together keeps a `--frozen-lockfile` install reproducible;
removing the `reborn-tests.yml` job must not orphan a required status check
(the job should be additive and non-required until deliberately promoted);
deleting `.claude/rules/*` immediately stops those rules auto-loading, which is
the intended effect. Each phase is independently revertible in this way.

**After any rollback, verify no dangling references remain** — use a
**hidden-file-aware** search, because plain `rg` skips dotfiles/dirs and
gitignored paths by default and would miss exactly the `.claude/` and `.github/`
locations the rollback touches. Search for **every exact path in the rollback
lists** (including the lower-case rule files and the workflow):

```bash
rg --hidden --glob '!.git' -n \
  'DESIGN\.md|CRITICAL_FLOWS|docs/features|\.storybook|design\.md|feature-workflow\.md|critical-flows\.md|reborn-tests\.yml'
```

Then strip any pointer left in `CLAUDE.md`, other `.claude/rules/*`, the CI
workflows, or docs, so a deleted file is never still referenced as authoritative.

## Open questions for reviewers

1. **`DESIGN.md` at repo root or `docs/design/`?** (Plan assumes `docs/design/`.)
2. **Feature-docs home** — `docs/features/<slug>/` vs. folding into `docs/plans/`?
3. **Chromatic** — is a paid visual-regression service in scope, or stay on the
   free Vitest/a11y subset?
4. **Scope of the docs-first `paths:`** — frontend only, or include specific
   product-surface crates?
5. **Who owns `DESIGN.md`** long-term (design vs. frontend eng)?
