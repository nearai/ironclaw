# IronClaw Brand Token Spec

The cross-surface deliverable from the Jul 17 product design review: the
brand-relevant design tokens, their semantics, and the mechanics another
NEAR surface — **near.com**, **market.near.ai**, **Datrix** — uses to adopt
(or map against) the IronClaw design language so the ecosystems stay
coherent.

Sources of truth:

- **Values**: `frontend/packages/design-system/src/tokens.css`
  (`@ironclaw/design-system/tokens.css`) — light + dark themes.
- **Catalog**: `tokens.ts` in the same package — machine-readable index,
  importable (`@ironclaw/design-system/tokens`).
- **Rendered spec**: the hosted Storybook (Tokens/Reference + Docs/Brand
  token spec) and the WebUI `/playground`.
- **Rules & provenance**: `DESIGN_SYSTEM.md` (this directory).

## 1. Model

Every token is a CSS custom property named `--v2-<role>`, **semantic by
role, never by palette** (`--v2-accent`, not `--v2-blue-500`). Light values
live on `:root`; dark overrides under `:root[data-theme="dark"]`.
Structural scales (radius, control density, spacing, type, tracking,
motion, z-index) are theme-independent. Components reference tokens
exclusively — re-theming a surface is a token file, zero component changes
(proof: the `soft` exploration theme, `themes/soft.css`).

A surface adopts the system by:

1. importing `@ironclaw/design-system/tokens.css` (or mirroring the
   variables it needs under its own build),
2. drawing every color from a semantic role,
3. picking steps from the structural scales instead of restating values,
4. theming via `data-theme` on the root element.

## 2. Brand pillars

| Pillar | Token(s) | Light | Dark |
|---|---|---|---|
| Brand blue (action) | `--v2-accent` | `#2882c8` | `#4ca7e6` |
| Accent pressed | `--v2-accent-strong` | `#1f6ca8` | `#2882c8` |
| Accent text/links | `--v2-accent-text` | `#2882c8` | `#6bb8ec` |
| Accent tint | `--v2-accent-soft` | `rgba(40,130,200,.1)` | `rgba(76,167,230,.15)` |
| Primary action ramp | `--v2-btn-primary-bg` / `-hover` | radial gradient `#4ca7e6 → #2882c8` (both themes) | same |
| Hover glow | `--v2-shadow-accent-hover` | `0 24px 24px -20px rgba(76,167,230,.55)` | same |
| Ink / brand mark | `--v2-ink` | `#090909` | `#4ca7e6` |
| Text on accent | `--v2-on-accent` | `#ffffff` (both themes) | same |

The **primary-button language** (radial blue ramp + hover glow +
translucent-outline secondary) is ironclaw.com parity and the single most
recognizable brand element — surfaces that adopt nothing else should adopt
`--v2-btn-*`.

## 3. Color roles (full catalog)

### Canvas & surfaces

| Token | Light | Dark | Use |
|---|---|---|---|
| `--v2-canvas` | `#f5f5f7` | `#09090b` | app background |
| `--v2-canvas-strong` | `#ffffff` | `#060608` | deepest background (body, chrome) |
| `--v2-surface` | `#ffffff` | `#0f0f11` | raised surface (panels, sidebar) |
| `--v2-surface-soft` | `#f5f5f7` | `rgba(255,255,255,.04)` | hover fills, subtle insets |
| `--v2-surface-muted` | `#ebebed` | `rgba(255,255,255,.07)` | strong insets, chips, skeletons |
| `--v2-rail` | `#f5f5f7` | `#0f0f11` | rails / input wells |
| `--v2-input-bg` | `#ffffff` | `#111113` | form control background |
| `--v2-code-bg` | `#f0f0f2` | `#111113` | inline code + code blocks |

### Cards & borders

| Token | Light | Dark | Use |
|---|---|---|---|
| `--v2-card-bg` | `#ffffff` | `#1b1b1f` | card/panel surface |
| `--v2-card-border` | `rgba(0,0,0,.08)` | `rgba(255,255,255,.08)` | card hairline |
| `--v2-panel-border` | `rgba(0,0,0,.08)` | `rgba(255,255,255,.1)` | default border: dividers, forms |

### Text

| Token | Light | Dark | Use |
|---|---|---|---|
| `--v2-text` | `#1a1a2e` | `#e0e0e0` | body |
| `--v2-text-strong` | `#090909` | `#fafafa` | headings, values |
| `--v2-text-muted` | `#555555` | `#a1a1aa` | secondary |
| `--v2-text-faint` | `#999999` | `#888888` | placeholders, eyebrows |
| `--v2-inverse` | `#ffffff` | `#111111` | text on inverse fills |

### Semantic status (one hue per status — see STATUS_CANON)

| Status | Text token | Fill token | Light text | Dark text |
|---|---|---|---|---|
| success / completed | `--v2-positive-text` | `--v2-positive-soft` | `#059669` | `#34d399` |
| running / active | `--v2-info-text` | `--v2-info-soft` | `#2563eb` | `#60a5fa` |
| warning / degraded | `--v2-warning-text` | `--v2-warning-soft` | `#d97706` | `#f5a623` |
| failure / error | `--v2-danger-text` | `--v2-danger-soft` | `#dc2626` | `#e64c4c` |
| paused / idle | `--v2-text-muted` | `--v2-surface-soft` | — | — |

### Glass & scrims

`--v2-glass-bg` + `--v2-glass-blur` (topbar/floating chrome),
`--v2-scrim` (`rgba(0,0,0,.55)`, modals), `--v2-scrim-soft`
(`rgba(0,0,0,.3)`, side sheets).

## 4. Typography

| Token / var | Value | Use |
|---|---|---|
| `--font-sans` | Geist | UI default |
| `--font-mono` | Geist Mono | data, code, numerics |
| `--font-serif` | Newsreader | editorial serif moments |
| `--v2-font-pixel` | Geist Pixel Square | the tag language (see below) |
| `--v2-font-size-*` | 11 / 12 / 13 / 14 / 16 / 20 / 24 / 28 / 36 px | label · caption · body-sm · body · body-lg · title · heading · display-sm · display |
| `--v2-tracking-*` | 0.08 / 0.14 / 0.22 / −0.02 / −0.04 em | tag · caps · wide · tight (headings) · display |

**The tag language** — small uppercase labels set in Geist Pixel Square at
11px/0.08em (`.v2-tag-face`) — is the second signature brand element
(ironclaw.com parity). Badges, eyebrows, and kickers use it everywhere.

## 5. Structure

- **Radii** `--v2-radius-*`: 6 / 8 / 10 / 16 / 20 / 24 / full. Buttons and
  inputs sit at `md` (10), cards at `lg` (16), modals at `2xl` (24).
- **Control density** `--v2-control-h-*`: 28 / 32 / 36 px heights with
  matched paddings — every interactive control aligns in mixed rows.
- **Spacing** `--v2-space-1…10`: the 4px grid, 4 → 40.
- **Elevation**: borders separate; shadows only lift. Themed scales
  (`--v2-shadow-sm/md/lg`, `--v2-card-shadow`, `--v2-shadow-menu`,
  `--v2-shadow-modal`) — the light scale is much softer than dark's.
- **Z-ladder** `--v2-z-*`: raised 10 · sticky 20 · overlay 40 · modal 50 ·
  toast 60.

## 6. Motion

Token-driven and restrained: durations `--v2-duration-*` (instant 100 ·
exit 120 · fast 150 · menu 180 · base 250 · slow 400 ms) paired with
easings `--v2-ease-*` (standard, in-out, out-expo for entrances, springs
for playful pops). Exits are quicker than entrances; nothing bounces on
pointer/keyboard input; the only ambient loops are typing / spin / breathe /
skeleton shimmer; everything respects `prefers-reduced-motion`.

## 7. Theming guidance for other surfaces

- **near.com / market.near.ai**: import `tokens.css` and restyle by
  exception. Marketing pages may lean on `--font-serif` display type and the
  outline-button language; product-y sections (pricing tables, dashboards)
  should consume the component package directly.
- **Datrix / sub-brands**: keep the structural scales and the status canon;
  swap the accent + neutrals under a `data-theme` of their own if brand
  separation is wanted. The `soft` theme
  (`@ironclaw/design-system/themes/soft.css`) is the worked example of a
  full re-skin via token overrides only.
- **Legacy Tailwind palettes**: the `@theme` block in `tokens.css` maps
  `iron-* / signal / copper / mint` utilities onto semantic tokens — a
  migration bridge, not an API for new code.

## 8. Open decisions (for the brand jam)

1. Typeface: Geist is the working default; flair vs. neutrality undecided.
2. Icon direction: in-house stroke set + lucide; same question.
3. IronClaw ↔ NEAR AI brand relationship (and its implications for accent,
   tag language, and near.com/near.ai design-language alignment).
4. How far product/marketing surfaces move toward the softer Claude-like
   warmth (the `soft` theme is the concrete artifact to react to).
