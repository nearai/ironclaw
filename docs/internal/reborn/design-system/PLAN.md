# Design System — Execution Plan (Phased)

**Status:** Proposal, under review · **Authored against:** `origin/main` @ `d3791e0f8` · **Tracks:** three Epics — [ownership table](README.md#epic-ownership-canonical)

This is the **when and how**; [CHECKLIST.md](CHECKLIST.md) is the **what** (definition of done); [PROPOSAL.md](PROPOSAL.md) is the frozen decision record. The five phases are **predefined** and executed in order; each phase heading links its owning Epic into the canonical table in [README.md](README.md#epic-ownership-canonical). Nothing here is sacred except the ordering constraints marked **⚠**.

> **Epic ownership** lives in one place: the canonical table in
> [README.md](README.md#epic-ownership-canonical). Every `Epic #NNNN` label below is a
> **link into that table**, not a restatement of it — this document holds no second copy
> of the mapping, so an ownership change is one edit, in the README.

**Operating principles:**
1. **Docs first** — each phase updates `DESIGN.md` / this package before or with the code (APDD Rule 1).
2. **Token values before component restyle** — Phase 3 lands tokens, validated in Storybook, before any primitive is reskinned.
3. **A story travels with every component change**; `pnpm test:storybook` stays green.
4. **`main` stays shippable** — phases land as reviewable PRs, stacked only where necessary.

```mermaid
flowchart LR
  P1["Phase 1 · Storybook · #7750"] --> P2["Phase 2 · DESIGN.md · #7042"] --> P3["Phase 3 · Theme & reskin"] --> P4["Phase 4 · Interactions"] --> P5["Phase 5 · IA"]
```

*The diagram shows sequencing only. Which Epic carries which phase is in the
[canonical table](README.md#epic-ownership-canonical) — it is deliberately not
redrawn here, so the grouping cannot drift out of step with it.*

## Phase 1 — Storybook integration (PR #7750 — in review) · [Epic #7038](README.md#epic-ownership-canonical)
*Stand up the workbench + catalog.* Storybook 10 (react-vite, pnpm), wired to `app.css` + light/dark toolbar; ~33 stories in five categories; vitest split (`pnpm test` node-only, `pnpm test:storybook` in Chromium); addon-mcp.
**Milestone:** catalog live; story + node suites green at the `crates/product/ironclaw_webui/frontend` path (103 story tests · 1355 node tests on #7750).
**Note:** originally PR #7039 — closed and recreated as **#7750**, clean and non-stacked off current `main`.

## Phase 2 — DESIGN.md governance & guidelines (issue #7042) · [Epic #7781](README.md#epic-ownership-canonical)
*Make the design system governed.* `DESIGN.md` (M3X spec + IronClaw appendix) and a Storybook `Design/Guidelines` page. **Precedence:** `AGENTS.md` is the canonical, tool-neutral agent contract — `DESIGN.md` is reachable from it, and `.claude/rules/design-system.md` plus the `CLAUDE.md` Module Specs pointer are Claude-specific adapters that *supplement* it, never replace or restate it.
**Milestone:** source of truth + agent rules in place; build-storybook green.
**⚠ Ordering:** the original #7043 was stacked on #7039; both were closed for the resulting merge tangle. Merge **#7750** first, then land the Phase-2 changeset as a fresh PR off `main` (PROPOSAL §7.6).

## Phase 3 — Theme update & UI reskin · [Epic #7781](README.md#epic-ownership-canonical)
*Change token values + assets to the M3X look; validate in Storybook.*
- Resolve the Phase-3 dependencies first — there are **two**, not three: **dark-palette derivation with WCAG AA contrast validation** (§7.3 — contrast is a standing invariant carried inside the palette work, not a separately-ownable dependency) and **fonts/licensing** (§7.4). Expressive motion is §7.5 and gates Phase 4, not this one.
- Land M3 → `--v2-*` token *values* (light + dark) in `app.css`; refresh color/type/space/radius scales; vendor fonts.
- Reskin primitives/composites against the new tokens; every change validated by its story + `CssCheck` + a11y.
- **Close the invariant-2 gap** (PROPOSAL §3.1): migrate the **345 occurrences of arbitrary pixel classes across 91 files** to the new type/space/radius scales, starting with the 38 occurrences in `design-system/`'s 8 files and the OOBE pilot card, and retire the **10 hardcoded six-digit hex values in 3 `.tsx` files**.
**Milestone:** new palette live in both themes, all `Tokens/*` stories pass contrast, primitives reskinned with green story tests, and `design-system/` free of arbitrary px/hex.
**Exit criteria (safeguards, PROPOSAL §7.0):** token *names* unchanged; fonts self-hosted with a tested fallback stack; the phase revertable by a single PR revert with no residue.
**⚠ Ordering:** Phase 2 lands **before** Phase 3 (DESIGN.md is the spec the token values are judged against, and both phases sit under the same Epic — see the [ownership table](README.md#epic-ownership-canonical)); token values land **before** component restyle; Phase 3 branches off `main` after #7750 and the Phase-2 PR merge.

## Phase 4 — Interaction & component updates (agentic-first) · [Epic #7782](README.md#epic-ownership-canonical)
*Add the expressive, agent-first interactions + new components.*
- Resolve the **animation approach** (reduced-motion-gated) and **MSW** for network-story happy-paths (PROPOSAL §7.2, §7.5).
- Build the agentic components (composer toolbar, FAB speed-dial, chat bubbles, agent-activity/reasoning cards, branded progress, connected button groups); each ships with stories + play coverage.
**Milestone:** agentic component set catalogued + story-tested; motion honors `prefers-reduced-motion`.
**Exit criteria (safeguards, PROPOSAL §7.0):** the production artifact asserted to contain neither `mockServiceWorker.js` nor an `msw` chunk (the worker never enters `public/` — PROPOSAL §7.2); motion opt-in per component, off via the single shared disabled-motion signal specified in PROPOSAL §7.5 — which cancels JS RAF loops, not just CSS — with the static baseline covered by caller-level tests.
**⚠ Ordering:** depends on Phase 3 tokens.

## Phase 5 — Information architecture · [Epic #7782](README.md#epic-ownership-canonical)
*Reshape navigation/routes/page structure to foreground agentic workflows.*
- Revisit `src/app/routes.ts`, `src/pages/`, the sidebar and `src/layout/gateway-layout.tsx` (paths relative to `crates/product/ironclaw_webui/frontend/`); adopt the M3X navigation-rail pattern where it fits; ensure multi-channel parity.
**Milestone:** IA restructured; CUJs (chat, approvals, projects, settings) verified unbroken.

## Suggested next PRs (concrete, in order)
1. **Merge #7750** — that closes Phase 1. Then land the Phase-2 (#7042) changeset as a fresh PR off `main`, opening Phase 2.
2. **Before Phase 3a opens — name the owners:** cut the dependency sub-issues on #7781 (dark palette + contrast, fonts/licensing **[decision]**) and assign them; PROPOSAL §7's accountability rule is what makes the gates real.
3. **Phase 3a — token foundation:** dark-palette + contrast + font vendoring in `app.css` (+ updated `Tokens/*` stories). No component restyle yet.
4. **Phase 3b — primitive reskin:** restyle `design-system/` primitives against the new tokens, story-by-story.
5. **Phase 4a — motion foundation:** choose + wire the animation approach behind the reduced-motion gate; add MSW.
6. **Phase 4b — first agentic component:** composer toolbar or agent-activity card, fully catalogued.

## Coordination notes
- This is a **docs-only** PR; it changes no code. It references the open Phase-1/2 PRs by number, and adds one cross-link to the OOBE package (PROPOSAL §9).
- **Design governance has one owner: this package** (PROPOSAL §9). OOBE's D-F6 contributes its card family as a pilot; it does not stand up a second `DESIGN.md`, token set, or workbench.
- Each phase is tracked under [its owning Epic](README.md#epic-ownership-canonical), with per-phase sub-issues (e.g. #7042) spun up as work starts. The closed Epic #7733 is recorded as superseded in that table.
- The APDD-kit evaluation (`docs/internal/apdd-governance-kit/`, [PR #7255](https://github.com/nearai/ironclaw/pull/7255) — open, not yet on `main`) is a sibling initiative that motivated this design-governance track; the two do not depend on each other.
