# Design System — Execution Plan (Phased)

**Status:** Proposal, under review · **Authored against:** `origin/main` @ `d3791e0f8` · **Tracks:** Epic [#7038](https://github.com/nearai/ironclaw/issues/7038)

This is the **when and how**; [CHECKLIST.md](CHECKLIST.md) is the **what** (definition of done); [PROPOSAL.md](PROPOSAL.md) is the frozen decision record. The five phases are **predefined** (Epic #7038) and executed in order. Nothing here is sacred except the ordering constraints marked **⚠**.

**Operating principles:**
1. **Docs first** — each phase updates `DESIGN.md` / this package before or with the code (APDD Rule 1).
2. **Token values before component restyle** — Phase 3 lands tokens, validated in Storybook, before any primitive is reskinned.
3. **A story travels with every component change**; `pnpm test:storybook` stays green.
4. **`main` stays shippable** — phases land as reviewable PRs, stacked only where necessary.

```mermaid
flowchart LR
  P1["Phase 1 · Storybook ✅ #7039"] --> P2["Phase 2 · DESIGN.md ✅ #7043"] --> P3["Phase 3 · Theme & reskin"] --> P4["Phase 4 · Interactions"] --> P5["Phase 5 · IA"]
```

## Phase 1 — Storybook integration ✅ (LANDED — PR #7039)
*Stand up the workbench + catalog.* Storybook 10 (react-vite, pnpm), wired to `app.css` + light/dark toolbar; ~33 stories in five categories; vitest split (`pnpm test` node-only, `pnpm test:storybook` in Chromium); addon-mcp.
**Milestone:** catalog live; 106 story tests + 1021 node tests green.

## Phase 2 — DESIGN.md governance & guidelines ✅ (LANDED — PR #7043)
*Make the design system governed.* `DESIGN.md` (M3X spec + IronClaw appendix), Storybook `Design/Guidelines` page, `.claude/rules/design-system.md`, `CLAUDE.md` Module Specs pointer.
**Milestone:** source of truth + agent rules in place; build-storybook green.
**⚠ Ordering:** #7043 is stacked on #7039 — merge #7039, then retarget #7043 to `main` (PROPOSAL §7.6).

## Phase 3 — Theme update & UI reskin
*Change token values + assets to the M3X look; validate in Storybook.*
- Resolve the Phase-3 dependencies first: **dark palette derivation**, **WCAG contrast validation**, **fonts/licensing** (PROPOSAL §7.3–§7.5).
- Land M3 → `--v2-*` token *values* (light + dark) in `app.css`; refresh color/type/space/radius scales; vendor fonts.
- Reskin primitives/composites against the new tokens; every change validated by its story + `CssCheck` + a11y.
**Milestone:** new palette live in both themes, all `Tokens/*` stories pass contrast, primitives reskinned with green story tests.
**⚠ Ordering:** token values land **before** component restyle; Phase 3 branches off `main` **after** #7039 + #7043 merge.

## Phase 4 — Interaction & component updates (agentic-first)
*Add the expressive, agent-first interactions + new components.*
- Resolve the **animation approach** (reduced-motion-gated) and **MSW** for network-story happy-paths (PROPOSAL §7.2, §7.5).
- Build the agentic components (composer toolbar, FAB speed-dial, chat bubbles, agent-activity/reasoning cards, branded progress, connected button groups); each ships with stories + play coverage.
**Milestone:** agentic component set catalogued + story-tested; motion honors `prefers-reduced-motion`.
**⚠ Ordering:** depends on Phase 3 tokens.

## Phase 5 — Information architecture
*Reshape navigation/routes/page structure to foreground agentic workflows.*
- Revisit `app/routes.ts`, `pages/`, sidebar / `gateway-layout`; adopt the M3X navigation-rail pattern where it fits; ensure multi-channel parity.
**Milestone:** IA restructured; CUJs (chat, approvals, projects, settings) verified unbroken.

## Suggested next PRs (concrete, in order)
1. **Merge #7039**, then retarget + merge **#7043** (unblocks Phase 3).
2. **Phase 3a — token foundation:** dark-palette + contrast + font vendoring in `app.css` (+ updated `Tokens/*` stories). No component restyle yet.
3. **Phase 3b — primitive reskin:** restyle `design-system/` primitives against the new tokens, story-by-story.
4. **Phase 4a — motion foundation:** choose + wire the animation approach behind the reduced-motion gate; add MSW.
5. **Phase 4b — first agentic component:** composer toolbar or agent-activity card, fully catalogued.

## Coordination notes
- This is a **docs-only** PR; it changes no code. It references the open Phase-1/2 PRs by number.
- Each later phase gets its **own sub-issue** under Epic #7038, spun up when the phase starts (per the Epic's tracking decision).
- The APDD-kit evaluation (`docs/plans/apdd-governance-kit/`, PR #7255) is a sibling initiative that motivated this design-governance track; the two do not depend on each other.
