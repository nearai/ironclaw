# @ironclaw/design-system

IronClaw's design system: Radix-based, shadcn-patterned React components, the
canonical `--v2-*` design tokens, and the restrained motion vocabulary. One
import surface for every IronClaw UI — the WebUI v2 agent workspace consumes
it today, and any future repository (marketing, sub-UIs, third-party hosts)
can import the same package for brand-consistent UI.

- **Components** — Button, Input/Textarea/Select/FormField, Card, Badge/StatusPill,
  Modal/ConfirmDialog, DropdownMenu, SelectMenu, Popover, Tooltip, Tabs,
  Checkbox, Switch, RadioGroup, Slider, Avatar, Separator, Skeleton,
  ScrollArea, Spinner, Icon, and layout primitives (Panel, SectionHeader,
  StatCard, FlowList, EmptyPanel, SubLabel). All interactive components sit
  on Radix primitives (keyboard navigation, focus management, ARIA) and are
  styled with Tailwind classes bound to `--v2-*` tokens.
- **Tokens** — `tokens.css` carries every `--v2-*` custom property (light +
  dark themes via `[data-theme]`) plus the Tailwind v4 `@theme` mapping.
  `tokens.ts` is the machine-readable index of the same tokens (powers the
  playground and Storybook token pages, and gives agents one import to
  enumerate the system).
- **Motion** — `motion.ts` exposes the duration/easing constants and
  `useReducedMotion`; the values mirror the `--v2-duration-*` / `--v2-ease-*`
  tokens.

## Documentation

- **Storybook** (canonical spec): every component with variants, props
  controls, a11y checks, and source exploration — `pnpm storybook` here, or
  the hosted instance linked from the PR.
- **`/playground`** in the WebUI: the same components rendered inside the
  real app shell — the in-product testbed.
- **`DESIGN_SYSTEM.md`** (in `crates/ironclaw_webui_v2/`): rules, provenance,
  and do/don't guidance.

## Installation

### Inside this repo (workspace)

The frontend app already depends on it via pnpm workspaces:

```jsonc
// crates/ironclaw_webui_v2/frontend/package.json
"dependencies": { "@ironclaw/design-system": "workspace:*" }
```

### From another repository

Until the package is published to a registry, consume it directly from git
or link it locally:

```bash
# git dependency (subdirectory)
pnpm add "https://gitpkg.vercel.app/nearai/ironclaw/crates/ironclaw_webui_v2/frontend/packages/design-system?main"

# or link a local checkout for development (symlink)
pnpm link /path/to/ironclaw/crates/ironclaw_webui_v2/frontend/packages/design-system
# or in package.json:
"@ironclaw/design-system": "file:../ironclaw/crates/ironclaw_webui_v2/frontend/packages/design-system"
```

Peer requirements: React 19. The package is **source-first** — its `exports`
point at TypeScript source, so a bundler that compiles TSX from
`node_modules` (Vite does out of the box) is expected. For hosts that can't,
`pnpm build` emits a compiled ESM bundle + type declarations in `dist/`.

## Setup in a consumer

1. **Tokens** — import once, before any component renders:

   ```css
   /* Tailwind v4 host (recommended) */
   @import "tailwindcss";
   @import "@ironclaw/design-system/tokens.css";
   @source "../node_modules/@ironclaw/design-system/src";
   ```

   The `@source` line tells Tailwind to compile the classes used inside the
   package components. Non-Tailwind hosts can import `tokens.css` too — the
   `@theme` at-rule is ignored outside Tailwind and every `--v2-*` custom
   property still applies (pair it with the `dist/` bundle).

2. **Fonts** — the token stacks reference Geist / Geist Mono / Newsreader
   (and Geist Pixel Square for tag labels). Hosts vendor their own webfonts;
   the system degrades gracefully to the fallback stacks.

3. **Theme** — set `data-theme="light" | "dark"` on `<html>` (or use the
   exported `useInterfaceTheme` hook, which persists to localStorage and
   respects `prefers-color-scheme`).

4. **Localization (optional)** — `Modal` and `ConfirmDialog` render built-in
   strings ("Close", "Cancel"). English is the default; localized hosts
   bridge their own translator:

   ```tsx
   import { DesignSystemI18nProvider } from "@ironclaw/design-system";

   <DesignSystemI18nProvider t={t}>
     <App />
   </DesignSystemI18nProvider>
   ```

## Usage

```tsx
import { Button, Card, CardBody, Modal, SelectMenu } from "@ironclaw/design-system";
import { COLOR_TOKENS, STATUS_CANON } from "@ironclaw/design-system/tokens";
import { MOTION_DURATION } from "@ironclaw/design-system/motion";
```

## Rules of the road

- Colors come from `--v2-*` tokens — never raw hex/rgb (the repo's
  `scripts/check-design-tokens.mjs` ratchet enforces this).
- Control heights come from the `--v2-control-h-*` scale (28/32/36).
- Motion uses the `--v2-duration-*` / `--v2-ease-*` pairs; all motion is
  suppressed under `prefers-reduced-motion`.
- Status colors follow `STATUS_CANON` in `tokens.ts` — one mapping from
  status words to semantic tokens, never a second hue.

See `DESIGN_SYSTEM.md` for the full guidance.
