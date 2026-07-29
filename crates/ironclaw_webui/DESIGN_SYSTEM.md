# IronClaw WebUI v2 Design System

The rules that make small UI changes safe to delegate. This document
exists so that an agent (or a human in a hurry) can make a micro-level
design decision — a spacing tweak, a new button, a status chip —
without a design review, because the decision space is already
constrained. Deep design work still goes through design; everything
here is the delegated layer.

## Provenance & reconciliation

Token values codify the **new aesthetic** developed in the design
explorations, not the legacy webui styling. Where they disagree, the
explorations win:

1. **`achal/nux` (gateway restyle)** — primary source. Backfilled
   from its `theme.css` + surface CSS: the zinc-dark palette
   (`#09090b / #0f0f11 / #1a1a1e`), light ink line
   (`rgba(0,0,0,0.08)`), brand-blue action ramp and 16px button
   radius (ironclaw.com parity), Geist Pixel Square tag language,
   layered shadow scale (sm/md/lg + card/modal), and the restrained
   motion system (100/150/250/400ms durations, out-expo/spring
   easings).
2. **`private-assistant` `achal/chat-first` (onboarding prototype)**
   — corroborating source for the overall direction (quiet surfaces,
   soft cards, shimmer/streaming affordances).

Reconciliation decisions made here:

- **Accent:** nux's IronClaw brand blue (`#4CA7E6` dark /
  `#2882c8` light) wins over the prototype's monochrome
  black-primary — the prototype is a generic shadcn base; nux is
  branded and more recent.
- **Radii:** nux's 6/8/10/16/24 scale wins over the prototype's
  `0.625rem`-derived shadcn scale and over legacy v2's 14px
  controls. A 20px `xl` step is kept between 16 and 24 because
  existing v2 cards render it.
- **Motion:** nux's token-driven restrained motion replaces legacy
  v2's global `animation: none` freeze. The three ambient loops the
  legacy UI allowed (typing/spin/breathe) are retained as the only
  ambient animations, plus the skeleton shimmer both explorations
  use.
- **Status colors:** nux's set (`#34d399 / #F5A623 / #E64C4C /
  #60a5fa` dark) wins over legacy v2's mint/gold/rose set.
- **Type scale:** nux's 11/13/14/16/20/24/36 gateway scale wins over
  legacy v2's ad-hoc 15px/1.2rem/1.35rem/2.2rem sizes; 12px caption
  and 28px stat sizes are kept from v2 components.

Second-round reconciliation with the onboarding-demo adoption
([PR #5565](https://github.com/nearai/ironclaw/pull/5565)), which
flagged four deliberate divergences:

- **Light shadow scale (adopted):** `--v2-shadow-sm/md/lg` are now
  **themed**, not universal. The demo's much softer light-mode scale
  (0.04–0.1 alpha, matching nux's own light theme block) is correct —
  dark-weight shadows read muddy on white. Dark keeps the original
  scale.
- **Code surface (new token):** `--v2-code-bg` is a distinct role
  from `--v2-input-bg`. They coincide in dark (`#111113`) but diverge
  in light (grey `#f0f0f2` code wash vs. white inputs), so forcing
  one onto the other was wrong. Markdown code styles now use it.
- **Soft scrim (new tokens):** `--v2-scrim-soft` (`rgba(0,0,0,0.3)`,
  the demo task-sheet scrim, for side sheets/panels) and `--v2-scrim`
  (`rgba(0,0,0,0.55)`, the modal dim — `Modal` now consumes it).
- **Third-party brand colors (rule, not tokens):** Gmail/Slack/
  Telegram etc. are intentionally outside the semantic system — see
  the brand-colors rule in §2.

Alias-shim audit (prompted by the `text-white` primary-button bug):
every compat alias was checked for literal-vs-semantic divergence.
Fixes: `--v2-on-accent` for text on brand fills (the shim's own
`bg-signal` rule now uses it too), `text-mint` re-aliased from the
blue accent to `--v2-positive-text` (legacy pages use mint only for
success/connected states — they rendered blue), and the hazardous
aliases are documented in §2 + flagged by the token ratchet, which
now also counts legacy alias utilities (grandfathered per file in
the baseline).

**Live reference:** open `/playground` in any running WebUI — it
renders every token and every component state below, in both themes.
(`/v2/playground` still works: the SPA strips the legacy `/v2` prefix.)

**Enforcement:** `node scripts/check-design-tokens.mjs` flags new
hardcoded colors outside the token system (see
[Enforcement](#enforcement) below). Run it before pushing UI changes.

---

## 1. Source of truth

| What | Where |
|---|---|
| Token values (color, light/dark) | `frontend/src/styles/app.css` — `--v2-*` custom properties |
| Token values (radii, spacing, type, shadows, motion, z) | `frontend/src/styles/app.css` — structural token block |
| Token index (machine-readable) | `frontend/src/design-system/tokens.js` |
| Components | `frontend/src/design-system/*.tsx` |
| Composites | `frontend/src/design-system/primitives.tsx` |
| Live gallery | `/playground` (`frontend/src/pages/playground/`) |

Everything else derives from these. If a value you need is missing,
add a token first — do not inline the raw value.

## 2. Color

- **Never write a hex/rgb color in page or component code.** Use
  `var(--v2-*)` (usually via a Tailwind arbitrary value, e.g.
  `text-[var(--v2-text-muted)]`). Every `--v2-*` color resolves for
  both themes automatically; a hardcoded color breaks one of them.
- Pick by **role**, not by appearance:
  - Backgrounds: `--v2-canvas` (page) → `--v2-surface` (panel) →
    `--v2-surface-soft` (hover/inset) → `--v2-surface-muted`
    (strong inset, code, skeleton).
  - Cards use the `--v2-card-*` triple via the `Card` component —
    don't rebuild it.
  - Text: `--v2-text` (body) / `--v2-text-strong` (headings, values) /
    `--v2-text-muted` (secondary) / `--v2-text-faint` (placeholders,
    eyebrows). Don't skip levels for contrast tricks.
  - Borders: `--v2-panel-border` everywhere unless you're inside
    `Card`.
  - Actions: `--v2-accent*`. Text sitting ON an accent/brand fill is
    `--v2-on-accent` (white in both themes) — never the `text-white`
    utility, which the legacy alias shim remaps to theme ink and
    renders dark-on-blue in light mode.
  - Status: `--v2-positive/warning/danger/info` pairs — always the
    `-soft` fill with the `-text` foreground, never `-text` on
    `-text`.
- **Status canon** (one hue per status, everywhere — text, dot, and
  progress fill all from the same pair; `STATUS_CANON` in
  `design-system/tokens.js` is the machine-readable copy):

  | Status words | Badge tone | Text/stroke | Fill |
  |---|---|---|---|
  | ok / success / completed | `success` | `--v2-positive-text` | `--v2-positive-soft` |
  | running / in progress / active | `info` | `--v2-info-text` | `--v2-info-soft` |
  | warning / degraded / attention | `warning` | `--v2-warning-text` | `--v2-warning-soft` |
  | failure / error / cancelled | `danger` | `--v2-danger-text` | `--v2-danger-soft` |
  | paused / idle / disabled | `muted` | `--v2-text-muted` | `--v2-surface-soft` |
- **Legacy alias utilities are forbidden in new code** (the ratchet
  flags them). The compat shim in app.css / the `@theme` block in
  index.html remap old utility classes to theme tokens, and several
  contradict their literal meaning — they are traps:
  - `text-white` / `hover:text-white` → renders `--v2-text-strong`
    (**dark ink in light mode**). On accent/brand fills use
    `text-[var(--v2-on-accent)]`; for headings use
    `text-[var(--v2-text-strong)]`.
  - `bg-white/*`, `border-white/*` → render surface-soft /
    panel-border, not a white veil. Never use them to lighten an
    image or gradient.
  - `bg-red-500` (solid) → renders the pale `--v2-danger-soft` tint,
    not a solid red fill. `bg-copper` (solid) → pale warning tint.
  - `text-mint` → `--v2-positive-text` (success green). It was
    mis-aliased to the blue accent until the alias audit; legacy
    pages use mint exclusively for success/connected states.
  - `iron-*`, `signal`, `copper`, `red-*` palette classes → theme
    tokens; write the `var(--v2-*)` form instead so intent is
    explicit.
- Code (inline + blocks) sits on `--v2-code-bg`, not `input-bg` or a
  surface tint — the roles diverge in light mode.
- **Third-party brand colors** (Gmail red, Slack aubergine, Telegram
  blue, provider logos, …) are deliberately *outside* the semantic
  token system: they are owned by the brands, don't theme, and must
  not be remapped. The rule:
  - Allowed **only** for rendering a third party's own mark, chip, or
    connect affordance — never for IronClaw UI meaning (a "danger"
    red is `--v2-danger-text`, even if it happens to look like
    Gmail's red).
  - They live in **one** brand-colors constant per surface (e.g. a
    `BRAND_COLORS` map in a `*-logos.js` / brand module, like
    `pages/onboarding/provider-logos.js`), never scattered inline.
  - The token ratchet still counts them; the brand module's
    occurrences are carried in `scripts/design-tokens-baseline.json`.
    Adding a new brand raises that one file's baseline in the same
    PR — a visible, reviewable diff — rather than being silently
    exempt.

## 3. Typography

- Fonts: Geist (sans, default), Geist Mono (data values, code), and
  Geist Pixel Square (the nux **tag face** — uppercase tags, badges,
  section kickers only, via the `.v2-tag-face` utility). Never
  introduce another font.
- Use the scale in `--v2-font-size-*` (nux gateway scale): `label`
  (11px) → `caption` (12) → `body-sm` (13) → `body` (14) → `body-lg`
  (16) → `title` (20) → `heading` (24) → `display-sm` (28) →
  `display` (36). If a size isn't on the scale, you don't need it.
- Small uppercase labels use `.v2-tag-face` (pixel face, 0.08em
  tracking). Mono-caps data eyebrows use `font-mono uppercase` with
  `--v2-tracking-caps` (0.14em) or `--v2-tracking-wide` (0.22em).
  Headings use negative tracking (`--v2-tracking-tight`, the nux
  heading tracking; `--v2-tracking-display` for oversized display).
- Weights: 400/500/600 only — headings are **medium (500)**, per the
  nux heading language. Numbers that align in columns get
  `tabular-nums`.

## 4. Spacing, layout & control density

- **Density principle:** the product is deliberately compact
  (Linear-density). Interactive controls draw their heights and
  horizontal paddings from the shared control tokens —
  `--v2-control-h-sm/md/lg` (28 / 32 / 36px) and
  `--v2-control-px-sm/md/lg` (10 / 12 / 16px) — so buttons, inputs,
  and selects align in mixed rows. New controls must consume these
  tokens; never invent a control height. When in doubt, pick the
  smaller size. (Buttons set this precedent; large marketing/hero
  CTAs on brand surfaces are the only place taller chrome is
  acceptable, and that's a design-review decision.)
- Everything sits on the 4px grid (`--v2-space-*`, equivalently the
  Tailwind default scale). No arbitrary `p-[13px]`-style one-offs.
- Relationships, not values: hairline gaps inside chips (4px),
  icon↔label (8px), between controls (12px), between form fields
  (16px), card padding (20px mobile / 28px desktop — but use
  `Card padding="md|lg"` instead of padding by hand), between
  sections (24px), page gutters (32px).
- Content max-widths: reading copy `max-w-[60ch]`-ish; forms
  `max-w-md`; don't let text lines run full-bleed.

## 5. Radii, shadows, z-index

- Radii come from `--v2-radius-*` (nux scale). Rule of thumb: the
  bigger and more container-like the element, the bigger the radius.
  Inline chips 6px, sm controls 8px, default controls (buttons,
  inputs) 10px, cards 16px, large cards 20px, modals and hero
  surfaces 24px, pills `full`. Never a new radius value. (The nux
  landing's 16px button radius is reserved for hero CTAs — at the
  compact control heights, 8–10px is the correct proportion.)
- Shadows follow a restrained elevation scale (all themed — the
  light scale is much softer, so never hardcode a shadow that
  "looks right" in one theme). Borders do the separation work;
  shadows only lift: `--v2-card-shadow` is a minimal 1px lift (via
  `Card`; use `variant="flat"` for in-page cards that should sit
  flush with no shadow at all), `--v2-shadow-menu` is shared by
  menus/popovers/tooltips/toasts, `--v2-shadow-modal` sits one step
  higher for dialogs, `--v2-shadow-sm/md/lg` cover other lifts, and
  `--v2-shadow-accent-hover` appears only on the primary button.
  Nothing else invents a shadow.
- Overlay dims come from the scrim tokens: `--v2-scrim` behind
  modals (what `Modal` renders), `--v2-scrim-soft` behind side
  sheets / task panels. Both sit on the overlay/modal layers below.
- Z-index is a five-layer ladder — `raised(10) → sticky(20) →
  overlay(40) → modal(50) → toast(60)`. Pick the layer that names
  your surface. Never invent a number between layers.

## 6. Motion

Motion follows the nux **restrained-motion** system: purposeful,
quick, and token-driven.

- Every duration comes from `--v2-duration-*` (`instant` 100ms for
  hover fills, `fast` 150ms for borders/small transforms, `base`
  250ms for panel/sheet entrances, `slow` 400ms for large surface
  transitions) and every easing from `--v2-ease-*` (`standard`,
  `in-out`, `out-expo` for entrances, `spring`/`spring-gentle` for
  small playful pops). **Never a raw ms value or ad-hoc
  cubic-bezier.**
- Ambient (infinite) animation is limited to work indicators:
  `.v2-typing-dot`, `.v2-spin`, the badge `v2-breathe` dot, and the
  `.v2-skeleton` shimmer.
- Entrances use `.v2-page-entrance` (or `base` + `out-expo`); don't
  choreograph multi-step sequences — that's a design-review
  decision.
- All motion is suppressed under `prefers-reduced-motion` by the
  global rule in app.css; never override it.

## 7. Components — when to use what

| Need | Use | Not |
|---|---|---|
| Any clickable action | `Button` | raw `<button>` with classes |
| The single main action on a surface | `Button variant="primary"` | multiple primaries side by side |
| Secondary / cancel action | `variant="secondary"` or `ghost` | a second primary |
| Destructive action | `variant="danger"` (+ confirm via `Modal` or `window.confirm`) | red text on a ghost button |
| Status chip | `Badge tone=…` with a **translated** `label` | hand-rolled pill |
| Panel / grouped content | `Card` (+ `CardHeader/Body/Footer/Label`) | `.v2-panel` (legacy shim) or bare bordered divs |
| Form control | `Input` / `Textarea` / `Select` inside `FormField` | raw inputs (they only get partial theming) |
| Toolbar select / sort control | `SelectMenu` (use `prefix` for the inline label) | an external label next to a bare select |
| Single-select view filter | `Tabs` (swap to `SelectMenu` below `sm`) | pill buttons or hand-rolled tab rows |
| Dialog / confirmation | `Modal` (+ `ModalBody/Footer`) | new overlay markup |
| Inline icon | `Icon name=…` (add glyphs to `icons.js`) | inline `<svg>` in a page |
| Metric, empty state, step list, section heading | `StatCard` / `EmptyPanel` / `FlowList` / `SectionHeader` / `SubLabel` | re-deriving them |

Component rules:

- **Buttons:** at most one `primary` (brand-gradient) per view
  section. `outline` is the nux secondary role — accent outline,
  frosted fill, fills solid on hover — for prominent secondary
  actions; `secondary`/`ghost` for quiet ones. Icon-only buttons
  must carry `aria-label`. Don't override a variant's colors via
  `className` — `className` is for layout (margin, width) only.
- **Badges:** tone conveys state (`success/warning/danger/info/
  accent/muted`); the label must be translated copy, not the tone
  keyword. Badges render in the pixel tag face (`.v2-tag-face`)
  automatically. The breathing dot on success tones is the only
  "live" indicator.
- **Forms:** every input gets a `Label` (via `FormField`); errors go
  in the `error` slot (rendered `role="alert"`), hints in `hint`.
  Mark required fields with `required` on `FormField`.
- **Modals:** one at a time; destructive confirms put the danger
  action right-most in `ModalFooter` with a `ghost` cancel.
- **i18n:** all user-facing product copy goes through `useT()` keys
  (see `lib/i18n.js`). The playground is the only exempt surface.

## 8. Rules for AI agents

You are expected to make small UI changes autonomously **within**
this system. That means:

**Allowed without review** — adding a field to a form using
`FormField`; adding a `Badge` state using an existing tone; a new
panel composed of `Card` + existing primitives; spacing fixes that
move a value *onto* the grid; adding an icon to `icons.js` matching
the 24px/1.7-stroke style; wiring an existing component into a page.

**Needs a design decision (stop and flag)** — a new color, font,
radius, shadow, duration/easing, or z-layer; a new component
category (e.g. tabs, tooltip, date-picker); multi-step motion
choreography; changing a token's value; changing a component's API
or default look; anything on the login/onboarding brand surfaces
beyond copy.

### Self-review checklist

Before committing a UI change, verify — honestly — each of:

1. **No raw values:** no new hex/`rgb()` colors, no off-grid px
   spacing, no raw ms durations or ad-hoc cubic-beziers, no new
   radius/shadow/z-index values.
   Run `node scripts/check-design-tokens.mjs` — it must pass.
2. **Right component:** the table above maps your need to a
   component; you used it rather than rebuilding it.
3. **Both themes:** every color you touched is a `--v2-*` var;
   check the change (or reason about it) in light *and* dark.
4. **States:** interactive elements have hover, focus-visible,
   and disabled handled (the components do this for you — another
   reason to use them).
5. **Motion policy:** any animation/transition you added uses the
   duration + easing tokens and follows the restrained-motion rules
   in §6 (feedback fast, entrances base/out-expo, no new ambient
   loops).
6. **A11y:** labels on inputs, `aria-label` on icon-only buttons,
   `role="alert"` for errors (via `FormField`), semantic headings.
7. **i18n:** new copy goes through i18n keys, and
   `scripts/check-i18n-parity.sh` still passes.
8. **Playground:** if you added a component variant/state, add it to
   the matching gallery in `frontend/src/pages/playground/` so the
   system stays browsable; if you added a token, register it in
   `design-system/tokens.js`.
9. **Checks:** `node --check` on every touched JS file, and the
   webui JS tests (`node --test` on the colocated `*.test.mjs`)
   still pass.

If any item fails and fixing it would require a new token/component/
motion, don't improvise — surface it for design review instead.

## 9. Enforcement

`scripts/check-design-tokens.mjs` scans
`crates/ironclaw_webui/frontend/src/**` (excluding vendor, dist, and
tests) for hardcoded hex / `rgb()` color literals — raw values are
only legitimate inside `frontend/src/styles/app.css`, where the tokens are
defined. Existing occurrences are grandfathered in
`scripts/design-tokens-baseline.json`; the check fails only when a
file *gains* raw colors. Reducing a file's count updates the
baseline via `--update-baseline` (do this in the same PR that removes
the violations, so the ratchet only ever goes down).

This script is the hook an agent reviewer runs to validate that a UI
PR stays inside the token system.
