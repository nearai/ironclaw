# Design System — Completion Checklist

**Status:** Proposal, under review · **Authored against:** `origin/main` @ `d3791e0f8` · **Tracks:** three Epics — [ownership table](README.md#epic-ownership-canonical)

**Definition of done:** when every box below is checked, the governed, agentic-first WebUI design system is fully realized. A checked box means **landed on `main`**, with the landing PR named inline. `⚠` marks a blocking prerequisite; `[decision]` marks an item gated on a named human call.

> **Epic ownership** lives in one place: the canonical table in
> [README.md](README.md#epic-ownership-canonical). Every `Epic #NNNN` label below is a
> **link into that table**, not a restatement of it — this document holds no second copy
> of the mapping, so an ownership change is one edit, in the README.

## WS1 — Storybook integration (Phase 1) · [Epic #7038](README.md#epic-ownership-canonical)
- [ ] Storybook 10 (`@storybook/react-vite`, pnpm) wired to `app.css` + light/dark toolbar — **Ships with #7750**
- [ ] ~33 stories in five categories (Primitives / Components / Composites / Icons / Tokens) — **#7750**
- [ ] Vitest split: `pnpm test` node-only, `pnpm test:storybook` in headless Chromium — **#7750**
- [ ] `@storybook/addon-mcp` available for agent access — **#7750**

## WS2 — DESIGN.md governance (Phase 2) · [Epic #7781](README.md#epic-ownership-canonical) · tracked by #7042
- [ ] `crates/product/ironclaw_webui/frontend/DESIGN.md` (M3X spec + IronClaw governance appendix) — **changeset preserved from closed #7043; fresh PR off `main`**
- [ ] Storybook `Design/Guidelines` docs page (`Design` sorts first) — **#7042**
- [ ] `.claude/rules/design-system.md` + `CLAUDE.md` Module Specs pointer + DS README link, all written as **supplements to `AGENTS.md`** — the canonical tool-neutral contract stays the entry point and the Claude files add no rule it does not carry — **#7042**
- [ ] ⚠ Merge #7750, then land the Phase-2 changeset as a fresh PR off `main` (PROPOSAL §7.6)

## WS3 — Theme foundation & reskin (Phase 3) · [Epic #7781](README.md#epic-ownership-canonical)
- [ ] Dark-palette values derived for every token (`:root[data-theme="dark"]`) (PROPOSAL §7.3)
- [ ] WCAG AA contrast validated for all text/token pairings; `Tokens/Colors` story asserts it — invariant §3.4, carried inside the dark-palette dependency (§7.3), not owned separately
- [ ] Fonts vendored: Roboto Flex + Roboto Mono (OFL) under `/vendor/fonts` — **[decision]** Google Sans substitute (§7.4)
- [ ] M3 → `--v2-*` token *values* land in `app.css` (light + dark)
- [ ] ⚠ Token values land **before** any component restyle
- [ ] Primitives/composites reskinned against new tokens; each story + `CssCheck` + a11y green
- [ ] **Invariant 2 gap closed** (PROPOSAL §3.1) — the **345 occurrences of arbitrary pixel classes across 91 files** migrated to the type/space/radius scales and the **10 hardcoded six-digit hex values in 3 `.tsx` files** retired; `design-system/` (38 occurrences in 8 files) and the OOBE pilot card go first, and a lint or grep gate keeps the count from regrowing
- [ ] **Safeguards (§7.0)** — fonts self-hosted with a tested system-fallback stack + `font-display: swap`; token *names* unchanged (values only); the whole phase revertable by a single PR revert with no residue

## WS4 — Agentic components & interactions (Phase 4) · [Epic #7782](README.md#epic-ownership-canonical)
- [ ] Animation approach chosen + wired behind a `prefers-reduced-motion` gate (PROPOSAL §7.5)
- [ ] MSW added for network-backed story happy-paths (PairingWebCodePanel, TeeShield) (§7.2)
- [ ] Agentic components built + catalogued (composer toolbar, FAB speed-dial, chat bubbles, agent-activity/reasoning cards, branded progress, connected button groups) — each with stories + play coverage
- [ ] **Safeguards (§7.0)** — production artifact asserted to contain neither `mockServiceWorker.js` nor an `msw` chunk, the worker generated into a Storybook-only static dir and never `frontend/public/` (§7.2); a missing handler degrades to the existing limited/error state
- [ ] **Motion kill switch (§7.5)** — one shared disabled-motion signal behind both `prefers-reduced-motion` and the app switch, read by CSS *and* every JS caller; a running spring cancels its RAF loop and writes the static end-state; a rejected dynamic motion chunk renders the static baseline; asserted by caller-level tests, not helper-level
- [ ] ⚠ Depends on WS3 tokens

## WS5 — Information architecture (Phase 5) · [Epic #7782](README.md#epic-ownership-canonical)
- [ ] Navigation/routes/page structure reshaped — `src/app/routes.ts`, `src/pages/`, the sidebar, `src/layout/gateway-layout.tsx` (relative to `crates/product/ironclaw_webui/frontend/`)
- [ ] M3X navigation-rail pattern adopted where it fits; multi-channel parity preserved
- [ ] Critical user journeys (chat, approvals, projects, settings) verified unbroken

## WS6 — Enforcement & CI (cross-cutting)
- [ ] `.claude/rules/design-system.md` governance kept current with each phase, and each update preserves the precedence: `AGENTS.md` canonical and tool-neutral, the Claude rule supplementary
- [ ] **[decision]** Optional CI job runs `playwright install chromium` + `pnpm test:storybook`; promote to a required gate only once stable (PROPOSAL §7.1) — **safeguards:** path-filtered to WebUI changes, non-blocking, and deletable without affecting any other lane
- [ ] `pnpm typecheck` + `pnpm lint:conventions` + `pnpm build-storybook` stay green each phase
- [ ] **Owners named before each gating phase opens** (PROPOSAL §7 accountability rule): a dependency sub-issue cut and assigned on the owning Epic for §7.1–§7.6, and every **[decision]** recorded with its caller on that Epic
- [ ] Design governance stays single-owner: no second `DESIGN.md`, token set, or Storybook workbench proposed outside this package (PROPOSAL §9)

## WS7 — Final verification gate (the 100% gate)
- [ ] Every primitive/composite/component with meaningful states has a story; `pnpm test:storybook` green
- [ ] Light + dark parity and WCAG AA contrast hold across the reskin
- [ ] The WebUI's theming, assets, interactions, and IA reflect the agentic-first principles in `DESIGN.md`
- [ ] `DESIGN.md` + the Storybook catalog are the demonstrated source of truth (new UI built through them)
- [ ] Each phase landed under [its owning Epic](README.md#epic-ownership-canonical)
