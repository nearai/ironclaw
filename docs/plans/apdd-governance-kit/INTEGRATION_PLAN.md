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
1–4 reference — and that **records the resolved `DESIGN.md` and feature-docs
paths, which the Rollback manifest then lists verbatim**. No code yet.

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
| 2.1 | Add Storybook via **pnpm** (`pnpm@11.7.0`, the repo's declared package manager) as **pinned `devDependencies`** — `storybook`, the **React/Vite framework adapter `@storybook/react-vite`**, the **browser-mode test provider `@vitest/browser` + `playwright`** (the addon runs axe/interaction in a real Chromium, not happy-dom), and the addons in 2.2 — commit the updated `pnpm-lock.yaml`, then run init/config. Storybook is a declared dependency of **neither `package.json` nor the lockfile** today, so treat this as a fresh, pinned install and **verify from a clean `pnpm install --frozen-lockfile`** — do not rely on any stray copy in the local (gitignored) `node_modules`. | `crates/product/ironclaw_webui/frontend` |
| 2.2 | Configure `.storybook/main.ts`: set **`framework: '@storybook/react-vite'`** (required for a React/Vite project) and register addons `@storybook/addon-vitest`, `-a11y`, `-docs`, `-mcp` (+ `@chromatic-com/storybook` deferred to Phase 4). Define the addon's **browser-mode Vitest project** — `@vitest/browser` with the `playwright` provider and `browser: { name: 'chromium' }` — as a project **separate** from the existing VM/happy-dom `vitest run`. | `.storybook/` + vitest config |
| 2.3 | `.storybook/preview.ts`: import the global stylesheet (tokens load) and wrap stories in the app theme provider decorator. **a11y mode:** covered stories run at **`test: 'error'`** (fail on any violation); only not-yet-triaged stories sit at `'todo'` — see the CI gate in 2.6. | `.storybook/preview.ts` |
| 2.4 | Author stories for **Tier-2 primitives first** (buttons, inputs, the items in `src/components/`): a smoke play test, a **token/CSS check** (computed style == token), and variant stories per visual state. | `src/**/*.stories.tsx` |
| 2.5 | Wire the **Storybook MCP** (`http://localhost:6006/mcp`) and add a CLAUDE.md / design-rule line: "query Storybook MCP `get-documentation` before using any component — never guess props." | `CLAUDE.md` + rule |
| 2.6 | Add the **Storybook a11y/test step to the `webui-v2-js-lint` job in `.github/workflows/code_style.yml`** — the actual WebUI frontend lane (it resolves the frontend dir via `scripts/ci/crate-dir.sh ironclaw_webui`, installs frozen deps, runs `pnpm test` = `vitest run` VM/happy-dom, and `pnpm build`, and is an aggregated **required** check). Because the Storybook addon runs in **browser mode** (`@vitest/browser` + Playwright Chromium), the job must also run **`pnpm exec playwright install --with-deps chromium`** (cache `~/.cache/ms-playwright`) before the browser project — separate from the existing VM-mode `pnpm test`. **Covered-story set (one definition, used everywhere):** every Phase-2-authored primitive story; Phase 2 exits only when that whole set runs at **`test: 'error'`** with zero axe violations. `test: 'todo'` prints nothing in CI and so cannot gate — it is reserved for stories authored by *later* work (post-Phase-2) pending their own triage, **never** a Phase-2 escape hatch. The gate runs the **full covered set** every run (emit axe results as a job artifact); changed-story filtering is a **local / pre-push fast path only**. | `.github/workflows/code_style.yml` (`webui-v2-js-lint`) |

**Exit:** `pnpm storybook` works **from a `--frozen-lockfile` install** with
Chromium provisioned; **every story in the covered set (all Phase-2 primitives)
passes axe at `test: 'error'`** in the `webui-v2-js-lint` CI job — no covered
story left at `'todo'`; the agent can introspect real props via MCP.

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
| 4.3 | Map each CUJ to its existing automated coverage (`reborn-e2e`, `reborn-playwright`) and flag any CUJ with no automation as a coverage gap. | `docs/qa/CRITICAL_FLOWS.md` |
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
feature docs or stories authored by other work). Paths below assume the Phase 0
defaults (§0.1 `docs/design/DESIGN.md`, §0.2 `docs/features/`); **if Phase 0
resolves a different location, record it in the decisions note and list that
exact path here instead**:
- `docs/design/DESIGN.md` *(or the §0.1-resolved location)*
- `.claude/rules/design.md` (and `design-a11y.md` if created), `.claude/rules/feature-workflow.md`, `.claude/rules/critical-flows.md`
- `docs/qa/CRITICAL_FLOWS.md`
- the `docs/features/_templates/` scaffold and the specific piloted `docs/features/<slug>/` folder(s) this rollout created *(or the §0.2-resolved location)* — **not** a blanket `docs/features/` delete
- the specific `.storybook/` files this rollout creates — `main.ts`, `preview.ts`, `vitest.setup.ts` (under `crates/product/ironclaw_webui/frontend/.storybook/`) — plus the specific `*.stories.tsx` files this rollout added. `.storybook/` does **not** exist in the frontend today, so removing the directory is safe **only after verifying it is still rollout-created** — otherwise delete just the listed files. **Not** every `*.stories.tsx`.

**Edits to existing files to revert with targeted patches** (revert only the
rollout's hunks — do not wholesale-revert a file that other work also touched):
- `CLAUDE.md` — the DESIGN.md pointer, the Storybook-MCP line, and the consolidated invariant list (Phase 1.4/1.5, 2.5)
- `crates/product/ironclaw_webui/frontend/package.json` + `pnpm-lock.yaml` — the pinned Storybook `devDependencies`
- `crates/product/ironclaw_webui/frontend/vite.config.ts` — the **browser-mode Vitest project** entry added in Phase 2.2 (or a new `vitest.config.ts` if split out — list it under added files in that case). Revert this together with the `@vitest/browser` / `playwright` deps so a rollback never leaves the browser project referencing removed packages.
- `.github/workflows/code_style.yml` — the Storybook a11y/browser **step added to the existing `webui-v2-js-lint` job** (remove the step; do **not** delete the job)

**Compatibility / hidden side effects:** reverting `package.json` +
`pnpm-lock.yaml` together keeps a `--frozen-lockfile` install reproducible; the
Storybook step is added to the **already-required** `webui-v2-js-lint` job, so
rollback removes just that step — it introduces no new required check and must
not disturb the job's existing `pnpm test` / `pnpm build` steps or the shared
`crate-dir.sh` dir resolution; deleting `.claude/rules/*` immediately stops
those rules auto-loading, which is the intended effect. Each phase is
independently revertible in this way.

**After any rollback, verify nothing stale survives** — two checks:

1. **No reference to a *deleted* file remains** — search for each removed path.
2. **No reference to a *reverted config/dep* remains** — after reverting the
   hunks in the edited files (`vite.config.ts`, `package.json`, `code_style.yml`,
   `CLAUDE.md`), confirm no Storybook / Vitest-browser leftovers linger (a
   half-reverted `vite.config.ts` still naming the browser project is the
   likeliest miss).

Use a search that is **hidden-file-aware and ignore-disabled**, because plain
`rg` skips dotfiles/dirs (`.claude/`, `.github/`) *and* gitignored build output
by default:

```bash
rg -uu --glob '!.git' --glob '!docs/plans/apdd-governance-kit/**' -n \
  'DESIGN\.md|CRITICAL_FLOWS|docs/features|\.storybook|design\.md|design-a11y\.md|feature-workflow\.md|critical-flows\.md|@vitest/browser|playwright|storybook'
```

(`-uu` = search hidden and gitignored files.) **Exclude the proposal docs
themselves** (`docs/plans/apdd-governance-kit/**`) — they enumerate every
removed path and dependency by design, so they would swamp the output with
expected self-matches. **Keep the edited production files in scope**
(`CLAUDE.md`, `package.json`, `vite.config.ts`, `code_style.yml`): after a
correct revert they name none of these, so a match *there* is a real leftover
(an incomplete revert) — precisely what this check should surface. As a
belt-and-suspenders complement, also confirm each edited file's reverted diff is
clean (per the manifest above).

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
