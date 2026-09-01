# design-system — UI primitives

Low-level, reusable UI building blocks for the WebUI v2 SPA. This is the
shadcn-style primitives layer (equivalent to `components/ui` in many React
codebases): small, presentational, dependency-light components that everything
else composes from.

Where it sits in the frontend:

```
design-system/  ← primitives (this folder)        e.g. Button, Input, Card
components/      ← shared composites built on them e.g. Sidebar, CommandPalette
layout/         ← the app frame / shell           e.g. GatewayLayout
pages/          ← feature views (routed)          e.g. ChatPage, SettingsPage
```

## Primitives

This table is a **map of the folder**, not an API reference: it says which file
owns which concern, so you know where to look. The canonical source of truth
for props, variants, and sizes is each component's own TypeScript signature —
and the live, interactive version is its Storybook entry (controls + docs).
Don't restate prop values here; they drift.

| File | Exports | Concern |
|------|---------|---------|
| `button.tsx` | `Button` | Buttons and button-styled links. |
| `switch.tsx` | `Switch` | Controlled toggle (`role="switch"`). |
| `input.tsx` | `Input`, `Textarea`, `Select`, `Label`, `FormField` | Form controls + labelled field wrapper with hint/error. |
| `select-menu.tsx` | `SelectMenu` | Custom popover select (richer than native `Select`). |
| `badge.tsx` | `Badge`, `StatusPill` | Small status chip with tone + optional dot. |
| `card.tsx` | `Card`, `CardHeader`, `CardBody`, `CardFooter`, `CardLabel` | Panel surface + composable sections. |
| `spinner.tsx` | `Spinner` | The single loading spinner (used by `Button loading`). |
| `icons.tsx` | `Icon`, `iconNames` | Inline SVG icon set + its canonical name list. |
| `modal.tsx` | `Modal`, `ModalHeader`, `ModalBody`, `ModalFooter` | Dialog/overlay. |
| `confirm-dialog.tsx` | `ConfirmDialog` | Confirm/cancel prompt built on `Modal`. |
| `primitives.tsx` | `Panel`, `StatCard`, `FlowList`, `EmptyPanel`, `SectionHeader`, `SubLabel`, `cx` | Higher-level composites built on `Card`/`Badge` (+ back-compat re-exports `StatusPill`, `Panel`). |
| `theme.ts` | `useInterfaceTheme`, `InterfaceTheme` | Light/dark theme hook (drives `data-theme` on `<html>`). |

## Design tokens

Every visual decision comes from a token in [`../styles/app.css`](../styles/app.css).
The file covers seven axes; `Tokens/*` in Storybook is the live sheet for each.

| Axis | Prefix | Storybook | Utility |
|------|--------|-----------|---------|
| Colour | `--v2-<role>` | `Tokens/Colors` | `text-iron-*`, `bg-signal`, … (theme-swapped) |
| Radius | `--v2-radius-*` | `Tokens/Shape & Space` | `rounded-chip`, `rounded-control`, … |
| Spacing | `--v2-space-*` | `Tokens/Shape & Space` | `p-inset`, `gap-stack`, … |
| Elevation | `--v2-elevation-*` | `Tokens/Shape & Space` | `shadow-e1`…`shadow-e3` (theme-swapped) |
| Stacking | `--v2-z-*` | `Tokens/Shape & Space` | `var()` — no Tailwind namespace |
| Type | `--text-ui-*`, `--text-title-*` | `Tokens/Typography` | `text-ui`, `text-title`, … |
| Motion | `--v2-duration-*`, `--v2-ease-*` | `Tokens/Motion` | `ease-standard`; durations via `var()` |

Three rules govern them:

- **Tokens are named for their role, never their size.** `--v2-radius-control`
  means "this is a button", so the reskin can change what a button looks like by
  editing one line. `--radius-md` would only relocate the hardcoding — the call
  site would still be asserting a size.
- **Reach for a token before an arbitrary value.** A `rounded-[14px]` or
  `shadow-[0_10px_30px_…]` in a component is a value the reskin cannot move. If
  no token fits, add one to `app.css` rather than inlining a literal.
- **Take durations from `--v2-duration-*`.** They collapse to `0ms` under
  `prefers-reduced-motion` at the token layer, so a component that uses one is
  reduced-motion-correct by construction. A hardcoded `duration-150` is not.

The static-motion policy in `app.css` still suppresses animation globally; the
motion tokens are the vocabulary Phase 4 (#7782 WS4) opts components back into,
not a licence to animate today. Token *values* are Phase 3a's to change
(#7781 WS3) — this table is the contract, not the palette.

### The contract is enforced against the built CSS, not the source

`pnpm build` runs `check:tokens`
([`../../scripts/check-token-bundle.ts`](../../scripts/check-token-bundle.ts)), which
reads `dist/assets/*.css` and fails when a token or utility named above is
missing from the *emitted* stylesheet.

It exists because everything else in this repo checks source. Component tests
assert Tailwind class **strings**, story tests assert rendered structure, and
`vite build` exits 0 whenever it produced a bundle — so all of them pass while
the CSS is silently wrong. That is not theoretical: a comment in `app.css`
containing a literal `*/` closed early and made Tailwind drop a whole `@theme`
block (29 keys). Every gate stayed green and no `rounded-control` utility was
emitted, which would have shipped every button square-cornered.

Two consequences when you touch `app.css`:

- **Adding a token before its first consumer needs `@theme static`.** Tailwind
  tree-shakes theme keys nothing references; plain `@theme` emits 41 of these
  keys, `static` emits 56.
- **Never write a bare `*/` inside a comment.** Spell utility groups
  `p-* m-* gap-*`, not `p-*/m-*/gap-*`.

The `Tokens/*` stories are the second half of the same guarantee: each sheet's
`play` function reads every property it documents and fails when one resolves
empty, and `Tokens/Motion` additionally asserts the
`prefers-reduced-motion` block zeroes all four durations.

## Conventions

- **One primitive per file.** Kebab-case filename, PascalCase export
  (`button.tsx` → `Button`). Import directly: `import { Button } from "../design-system/button"`.
- **Styling is Tailwind + CSS variables.** Components use Tailwind utilities
  backed by the `--v2-*` design tokens defined in `../styles/app.css`
  (`@theme` + `:root[data-theme="dark|light"]`). Do **not** reference legacy
  `app.css` component classes — light/dark theming is automatic via the tokens.
  Merge class names with `../utils/cn`.
- **Tests are colocated** as `*.test.tsx` / `*.test.ts` next to the component.
- **Stories are colocated** as `*.stories.tsx` next to the component (see below).

## Storybook

Stories live beside their component (`button.tsx` → `button.stories.tsx`). The
sidebar is organized into five top-level categories via an explicit `meta.title`
(**not** path-based auto-titles):

| Category | What lives there |
|----------|------------------|
| `Primitives/*` | atomic design-system components (Button, Input, Switch, Modal, SelectMenu…) |
| `Composites/*` | higher-level compositions (`primitives.tsx`: StatCard, EmptyPanel…) |
| `Components/*` | app-wired shared components (`components/`, `layout/`) — need providers |
| `Icons/*` | the `Icon` catalog |
| `Tokens/*` | color / typography / motion swatches read from `app.css` |

The order is set by `storySort` in `.storybook/preview.tsx`, which also imports
the real `app.css` and `i18n/en`, and exposes a light/dark toolbar so stories
render with production tokens and strings.

```bash
pnpm storybook                              # dev server on :6006
pnpm build-storybook                        # static build
pnpm test:storybook                         # run story tests (headless Chromium)
pnpm test                                   # node unit suite only (no browser)
```

> `pnpm test` runs only the Node unit project; the browser-based story suite is
> a separate `test:storybook` script so `pnpm test` needs no Playwright browser.

Adding a story:

```tsx
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Thing } from "./thing";

const meta = {
  title: "Primitives/Thing", // pick the right category
  component: Thing,
  tags: ["ai-generated"], // drop when hand-reviewed
} satisfies Meta<typeof Thing>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
```

Keep `play` functions for things the render alone doesn't prove (interactions,
state transitions, a computed-style check that the stylesheet loaded).

**App-wired `Components/*`** need context. Use the shared decorators in
[`src/test-support/storybook-decorators.tsx`](../test-support/storybook-decorators.tsx):
`withRouter()` (MemoryRouter) and `withQueryClient(seed)` (a fresh QueryClient,
optionally pre-seeded via `client.setQueryData(...)` so components render their
loaded state with no network). i18n needs no decorator — the preview imports the
`en` pack globally.
