# `@ironclaw/ui` — design-system package (scaffold)

Target home for the IronClaw WebUI design system as a first-class package,
scaffolded now as a **reference for the Phase-3 reskin** (Epic
[#7038](https://github.com/nearai/ironclaw/issues/7038)). This is the framework
the current `../../src/design-system/` primitives migrate **into** over Phases
3–4.

> **Status: inert scaffold — not yet wired.** Every file here is a stub. The
> package is deliberately **not** in `pnpm-workspace.yaml`, the Storybook stories
> glob, or `tsconfig.json`, so it neither builds nor runs today. Phase 3 wires it
> in (workspace member → tokens → components), one component at a time.

## Structure

```
packages/ui/src/
├── tokens/       colors · typography · spacing · radii · shadows · motion   (one .css per token layer)
├── themes/       light.css · dark.css · themes.stories.tsx                  (theme token values + preview)
├── icons/        icon.tsx · registry.ts · icons.stories.tsx · index.ts
├── components/   button · input · select · card · modal · badge · switch    (one folder each)
│   └── button/   button.tsx · button.variants.ts · button.stories.tsx · button.test.tsx · index.ts
└── styles/       index.css                                                  (package style entry: @imports the token + theme layers)
```

**Per-component convention** (see `components/button/` as the worked template):

| File | Role |
|------|------|
| `<name>.tsx` | The component — presentational; styled through tokens, never hardcoded hex/px |
| `<name>.variants.ts` | Variant/size map (cva-style) kept out of the component body |
| `<name>.stories.tsx` | Storybook catalog entry (smoke + variant + `play` where it proves something) |
| `<name>.test.tsx` | Unit test |
| `index.ts` | Public re-export |

## Migration mapping (Phase 3)

| Today (`src/design-system/`) | Target (`packages/ui/src/`) |
|---|---|
| `app.css` `@theme` + `--v2-*` (in `src/styles/`) | `tokens/*.css` + `themes/{light,dark}.css` |
| `button.tsx`, `input.tsx`, `select-menu.tsx`, `card.tsx`, `modal.tsx`, `badge.tsx`, `switch.tsx` | `components/<name>/` |
| `icons.tsx` | `icons/` |
| `*.stories.tsx` (colocated) | colocated in each `components/<name>/` |
| `src/test-support/storybook-decorators.tsx` | `.storybook/decorators.tsx` (shared) |

## Rules

Governed by [`DESIGN.md`](../../DESIGN.md) and
[`.claude/rules/design-system.md`](../../../../../../.claude/rules/design-system.md):
native M3X on React + Tailwind, token-driven (no hardcoded hex/px), a story per
component, WCAG AA + light/dark parity, `prefers-reduced-motion`-gated motion.
