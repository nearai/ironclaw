# WebUI Design System governance

The IronClaw WebUI has a **governed design system**. When you add or change any
UI in `crates/ironclaw_webui/frontend`, follow this rule. It exists so the
look, feel, interactions, and tokens stay coherent as the product moves toward
an AI/agentic-first UX (Epic #7038).

## Source of truth

- **`crates/ironclaw_webui/frontend/DESIGN.md`** is the design source of truth
  (the M3 Expressive design *language* + IronClaw governance appendix).
- The **Storybook catalog** (`src/**/*.stories.tsx`, categories
  Primitives / Components / Composites / Icons / Tokens) is the living
  reference and review surface.
- When DESIGN.md and the current code conflict on *look/feel*, DESIGN.md wins.
  When they appear to conflict on *implementation*, the Epic's non-goals win
  (see Precedence below).

## Native M3X — not Material Web

Realize the design language with the **existing React 19 + Tailwind v4
primitives**. Do **not** introduce Material Web (`<md-filled-button>`,
`<md-elevation>`, Lit web components) or any parallel/third-party component
framework. "M3 Expressive" is the visual/interaction system, applied with our
own components.

## Token-driven — no hardcoded values

- Never hardcode hex colors or raw px sizing in a component. Consume design
  tokens: `--v2-*` CSS variables and the `@theme` scale in
  `src/styles/app.css`, merged with the `cn()` helper.
- Need a new value? **Add a token** in `app.css` — under `@theme` and/or both
  `:root[data-theme="light"]` and `:root[data-theme="dark"]` — then reference
  it. The `data-theme` + `--v2-*` token architecture is kept; do not replace it.
- Every color token must define **light and dark** values, and pass **WCAG AA**
  contrast for any text pairing before use.

## Story-per-component

- Every primitive / composite / component with meaningful states gets a
  colocated `*.stories.tsx` (see `src/design-system/README.md` for authoring
  conventions and category titles).
- Visual and interaction changes are reviewed in Storybook and covered by the
  story suite: `pnpm test:storybook` (headless Chromium). Keep exactly one
  project-wide `CssCheck` proving the stylesheet loaded.
- Node unit tests stay browser-free: `pnpm test`.

## Accessibility bar

Preserve `aria-*` attributes, roles, and keyboard/focus behavior when
restyling — never strip them for aesthetics. Maintain visible focus states,
light + dark parity, and WCAG AA contrast.

## Motion policy

`app.css` ships a static-motion policy (`* { animation: none !important }`, with
`.v2-spin` the sole exception, `prefers-reduced-motion` honored). Expressive
motion (spring physics, shape-morph, speed-dial unfurl) is opt-in, **gated on
`prefers-reduced-motion`**, and introduced through an approved animation
approach — never ad-hoc CSS keyframes that bypass the policy.

## Precedence & phasing

- DESIGN.md governs *what it looks/feels like*; Epic #7038's non-goals govern
  *what we build it with* (no framework swap; no token-architecture change).
- Respect the phasing: **Phase 2** = this governance (DESIGN.md + Storybook docs
  + these rules). **Phase 3** = token values / reskin. **Phase 4** = new
  agentic components + interactions. **Phase 5** = information architecture.
  Do not pull later-phase reskin/components work into an unrelated change.

## Before you build a new UI element

1. Read `DESIGN.md` (spec + appendix) and the relevant `*.stories.tsx`.
2. Map the need to existing tokens/primitives; add tokens rather than inline
   values.
3. Ship the change with a story and run the validation gates
   (`pnpm typecheck && pnpm lint:conventions && pnpm test:storybook &&
   pnpm build-storybook`).
