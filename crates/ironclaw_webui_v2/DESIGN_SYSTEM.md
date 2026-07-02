# IronClaw WebUI v2 Design System

The rules that make small UI changes safe to delegate. This document
exists so that an agent (or a human in a hurry) can make a micro-level
design decision — a spacing tweak, a new button, a status chip —
without a design review, because the decision space is already
constrained. Deep design work still goes through design; everything
here is the delegated layer.

**Live reference:** open `/v2/playground` in any running WebUI — it
renders every token and every component state below, in both themes.

**Enforcement:** `node scripts/check-design-tokens.mjs` flags new
hardcoded colors outside the token system (see
[Enforcement](#enforcement) below). Run it before pushing UI changes.

---

## 1. Source of truth

| What | Where |
|---|---|
| Token values (color, light/dark) | `static/styles/app.css` — `--v2-*` custom properties |
| Token values (radii, spacing, type, shadows, motion, z) | `static/styles/app.css` — structural token block |
| Token index (machine-readable) | `static/js/design-system/tokens.js` |
| Components | `static/js/design-system/*.js` |
| Composites | `static/js/design-system/primitives.js` |
| Live gallery | `/v2/playground` (`static/js/pages/playground/`) |

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
  - Actions: `--v2-accent*`. Status: `--v2-positive/warning/danger/
    info` pairs — always the `-soft` fill with the `-text` foreground,
    never `-text` on `-text`.
- The legacy `iron-*` / `signal` / `copper` Tailwind aliases in
  `index.html` are compat shims for old pages. Do not use them in new
  code.

## 3. Typography

- Fonts: Geist (sans, default), Geist Mono (labels, values, code).
  Never introduce another font.
- Use the scale in `--v2-font-size-*`: `label` (11px mono-caps) →
  `caption` (12) → `body-sm` (13) → `body` (14) → `body-lg` (15) →
  `title` (1.2rem) → `heading` (1.35rem) → `display-sm` (1.75rem) →
  `display` (2.2rem). If a size isn't on the scale, you don't need it.
- Mono-caps labels (`font-mono uppercase`) always pair with
  `--v2-tracking-caps` (0.14em) or `--v2-tracking-wide` (0.22em for
  card eyebrows) and a muted/faint text color. Large headings use
  negative tracking (`--v2-tracking-tight` / `--v2-tracking-display`).
- Weights: 400/500/600 only. Numbers that align in columns get
  `tabular-nums`.

## 4. Spacing & layout

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

- Radii come from `--v2-radius-*`. Rule of thumb: the bigger and more
  container-like the element, the bigger the radius. Compact controls
  10px, default controls 14/16px (mobile/desktop), cards 20–24px,
  pills `full`. Never a new radius value.
- Shadows: cards use the themed `--v2-card-shadow` (via `Card`);
  modals use `--v2-shadow-modal`. Nothing else casts a shadow.
- Z-index is a five-layer ladder — `raised(10) → sticky(20) →
  overlay(40) → modal(50) → toast(60)`. Pick the layer that names
  your surface. Never invent a number between layers.

## 6. Motion

**The v2 UI is static by policy.** `app.css` globally disables all
CSS transitions and animations. The only sanctioned exceptions are
the three "work is happening" loops: `.v2-typing-dot`, `.v2-spin`,
and the badge `v2-breathe` dot — each suppressed under
`prefers-reduced-motion`. Do not add animation, transition, hover
motion, or entrance effects. If a feature genuinely needs motion,
that is a design-review decision, not a micro-decision.

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
| Dialog / confirmation | `Modal` (+ `ModalBody/Footer`) | new overlay markup |
| Inline icon | `Icon name=…` (add glyphs to `icons.js`) | inline `<svg>` in a page |
| Metric, empty state, step list, section heading | `StatCard` / `EmptyPanel` / `FlowList` / `SectionHeader` / `SubLabel` | re-deriving them |

Component rules:

- **Buttons:** at most one `primary` per view section. Icon-only
  buttons must carry `aria-label`. Don't override a variant's colors
  via `className` — `className` is for layout (margin, width) only.
- **Badges:** tone conveys state (`success/warning/danger/info/
  accent/muted`); the label must be translated copy, not the tone
  keyword. The breathing dot on success tones is the only "live"
  indicator.
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
radius, shadow, animation, or z-layer; a new component category
(e.g. tabs, tooltip, date-picker); changing a token's value; changing
a component's API or default look; anything on the login/onboarding
brand surfaces beyond copy.

### Self-review checklist

Before committing a UI change, verify — honestly — each of:

1. **No raw values:** no new hex/`rgb()` colors, no off-grid px
   spacing, no new radius/shadow/z-index/duration values.
   Run `node scripts/check-design-tokens.mjs` — it must pass.
2. **Right component:** the table above maps your need to a
   component; you used it rather than rebuilding it.
3. **Both themes:** every color you touched is a `--v2-*` var;
   check the change (or reason about it) in light *and* dark.
4. **States:** interactive elements have hover, focus-visible,
   and disabled handled (the components do this for you — another
   reason to use them).
5. **Motion policy:** you added no animation/transition.
6. **A11y:** labels on inputs, `aria-label` on icon-only buttons,
   `role="alert"` for errors (via `FormField`), semantic headings.
7. **i18n:** new copy goes through i18n keys, and
   `scripts/check-i18n-parity.sh` still passes.
8. **Playground:** if you added a component variant/state, add it to
   the matching gallery in `static/js/pages/playground/` so the
   system stays browsable; if you added a token, register it in
   `design-system/tokens.js`.
9. **Checks:** `node --check` on every touched JS file, and the
   webui JS tests (`node --test` on the colocated `*.test.mjs`)
   still pass.

If any item fails and fixing it would require a new token/component/
motion, don't improvise — surface it for design review instead.

## 9. Enforcement

`scripts/check-design-tokens.mjs` scans
`crates/ironclaw_webui_v2/static/js/**` (excluding vendor, dist, and
tests) for hardcoded hex / `rgb()` color literals — raw values are
only legitimate inside `static/styles/app.css`, where the tokens are
defined. Existing occurrences are grandfathered in
`scripts/design-tokens-baseline.json`; the check fails only when a
file *gains* raw colors. Reducing a file's count updates the
baseline via `--update-baseline` (do this in the same PR that removes
the violations, so the ratchet only ever goes down).

This script is the hook an agent reviewer runs to validate that a UI
PR stays inside the token system.
