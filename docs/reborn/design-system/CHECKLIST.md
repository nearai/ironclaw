# Design System — Completion Checklist

**Status:** Proposal, under review · **Authored against:** `origin/main` @ `d3791e0f8` · **Tracks:** Epic [#7038](https://github.com/nearai/ironclaw/issues/7038)

**Definition of done:** when every box below is checked, the governed, agentic-first WebUI design system is fully realized. A checked box means **landed on `main`**, with the landing PR named inline. `⚠` marks a blocking prerequisite; `[decision]` marks an item gated on a named human call.

## WS1 — Storybook integration (Phase 1)
- [x] Storybook 10 (`@storybook/react-vite`, pnpm) wired to `app.css` + light/dark toolbar — **Landed with #7039**
- [x] ~33 stories in five categories (Primitives / Components / Composites / Icons / Tokens) — **#7039**
- [x] Vitest split: `pnpm test` node-only, `pnpm test:storybook` in headless Chromium — **#7039**
- [x] `@storybook/addon-mcp` available for agent access — **#7039**

## WS2 — DESIGN.md governance (Phase 2)
- [x] `crates/ironclaw_webui/frontend/DESIGN.md` (M3X spec + IronClaw governance appendix) — **Landed with #7043**
- [x] Storybook `Design/Guidelines` docs page (`Design` sorts first) — **#7043**
- [x] `.claude/rules/design-system.md` + `CLAUDE.md` Module Specs pointer + DS README link — **#7043**
- [ ] ⚠ Merge #7039, then retarget #7043 to `main` (PROPOSAL §7.6)

## WS3 — Theme foundation & reskin (Phase 3)
- [ ] Dark-palette values derived for every token (`:root[data-theme="dark"]`) (PROPOSAL §7.3)
- [ ] WCAG AA contrast validated for all text/token pairings; `Tokens/Colors` story asserts it (§7.4/§8-a11y)
- [ ] Fonts vendored: Roboto Flex + Roboto Mono (OFL) under `/vendor/fonts` — **[decision]** Google Sans substitute (§7.4)
- [ ] M3 → `--v2-*` token *values* land in `app.css` (light + dark)
- [ ] ⚠ Token values land **before** any component restyle
- [ ] Primitives/composites reskinned against new tokens; each story + `CssCheck` + a11y green

## WS4 — Agentic components & interactions (Phase 4)
- [ ] Animation approach chosen + wired behind a `prefers-reduced-motion` gate (PROPOSAL §7.5)
- [ ] MSW added for network-backed story happy-paths (PairingWebCodePanel, TeeShield) (§7.2)
- [ ] Agentic components built + catalogued (composer toolbar, FAB speed-dial, chat bubbles, agent-activity/reasoning cards, branded progress, connected button groups) — each with stories + play coverage
- [ ] ⚠ Depends on WS3 tokens

## WS5 — Information architecture (Phase 5)
- [ ] Navigation/routes/page structure reshaped (`app/routes.ts`, `pages/`, sidebar / `gateway-layout`)
- [ ] M3X navigation-rail pattern adopted where it fits; multi-channel parity preserved
- [ ] Critical user journeys (chat, approvals, projects, settings) verified unbroken

## WS6 — Enforcement & CI (cross-cutting)
- [ ] `.claude/rules/design-system.md` governance kept current with each phase
- [ ] **[decision]** Optional CI job runs `playwright install chromium` + `pnpm test:storybook`; promote to a required gate only once stable (PROPOSAL §7.1)
- [ ] `pnpm typecheck` + `pnpm lint:conventions` + `pnpm build-storybook` stay green each phase

## WS7 — Final verification gate (the 100% gate)
- [ ] Every primitive/composite/component with meaningful states has a story; `pnpm test:storybook` green
- [ ] Light + dark parity and WCAG AA contrast hold across the reskin
- [ ] The WebUI's theming, assets, interactions, and IA reflect the agentic-first principles in `DESIGN.md`
- [ ] `DESIGN.md` + the Storybook catalog are the demonstrated source of truth (new UI built through them)
- [ ] Each phase landed via its own sub-issue under Epic #7038
