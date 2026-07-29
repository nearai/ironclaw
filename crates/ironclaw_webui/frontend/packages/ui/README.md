# @ironclaw/ui

The IronClaw WebUI design system. Consumed by the SPA (`crates/ironclaw_webui/frontend`)
as **TypeScript source** through the pnpm workspace, so editing a component here
hot-reloads instantly in a running `pnpm dev` session — no build or publish step.

## Layers

```
src/
  tokens/       tokens.css — CSS custom properties for light/dark themes,
                the semantic type scale, and sanctioned motion keyframes.
  primitives/   Leaf building blocks: cn, Icon, Spinner, Skeleton.
  components/   Themed controls: Button, IconButton, Badge, Card, Callout,
                Input/Textarea/Select/Label/FormField, SelectMenu, Modal.
  composites/   Higher-order assemblies: ConfirmDialog, Breadcrumb,
                EmptyPanel, StatCard, FlowList, SectionHeader/SubLabel.
  theme/        useInterfaceTheme (light/dark persistence) and the UiText
                provider that bridges the app's i18n into built-in strings.
```

Rules of the road:

- Lower layers never import from higher layers.
- All visual styling is Tailwind utility classes backed by the CSS variables
  in `tokens.css`; components carry no hard-coded palette values.
- The package is i18n-agnostic: user-visible strings come in through props;
  the few built-in fallbacks (modal close, confirm cancel) resolve through
  `UiTextProvider`.

## Extras

`src/extras/` holds a second, quarantined component kit: pre-built,
token-faithful components (shadcn/ui gap coverage — Tabs, Tooltip, Switch,
Combobox, DatePicker, Toast, …) that aren't used by any product surface yet.
They're exported only via the `@ironclaw/ui/extras` subpath, never from the
main barrel, so they cost nothing until adopted. See
[`src/extras/README.md`](./src/extras/README.md) for the inventory and the
promotion path into the core set.

## Usage

```tsx
import { Button, Card, ConfirmDialog } from "@ironclaw/ui";
```

Tokens are imported once by the app's `src/styles/app.css`:

```css
@import "@ironclaw/ui/tokens.css";
```

## Storybook

```sh
pnpm --filter @ironclaw/ui storybook        # dev server on :6006
pnpm --filter @ironclaw/ui build-storybook  # static build
```

Stories live in `stories/`, one file per component, with a light/dark theme
toolbar toggle mirroring the app's `data-theme` switching.

## Tests

Component tests are colocated with sources (`*.test.ts[x]`) and run as part
of the frontend suite: `pnpm test` from `crates/ironclaw_webui/frontend`.
