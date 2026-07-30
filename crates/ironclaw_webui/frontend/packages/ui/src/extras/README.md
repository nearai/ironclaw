# @ironclaw/ui/extras

Pre-built, token-faithful components that **no product surface uses yet**.
This is the design system's shadcn/ui gap coverage: everything on shadcn's
component list that the core set doesn't already provide, built ahead of need
so a new surface can adopt it without a design scramble.

```tsx
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@ironclaw/ui/extras";
```

## Ground rules

- **Separate barrel on purpose.** Extras are exported only from
  `@ironclaw/ui/extras`, never from the main `@ironclaw/ui` barrel. The app
  bundle pays for an extra only when a surface actually imports it.
- **Same quality bar as core.** Strict TypeScript, Tailwind utilities backed by
  the `--v2-*` tokens (both themes work automatically), type scale capped at
  semibold, motion limited to simple transitions, keyboard/a11y semantics from
  Radix or hand-implemented WAI-ARIA patterns.
- **Promotion path.** When a surface needs one of these, move the file into
  `src/components/` (or `src/composites/`), export it from `src/index.ts`, and
  delete it here. Stories (`stories/extras/`) and colocated smoke tests move
  with it.

## Contents

| Area | Components |
| --- | --- |
| Disclosure | Accordion, Collapsible |
| Layout | AspectRatio, Separator, ScrollArea, Resizable (react-resizable-panels) |
| Identity | Avatar |
| Forms | Checkbox, RadioGroup, Slider, InputOTP, Combobox, DatePicker + Calendar (Switch graduated to core `components/`) |
| Menus | DropdownMenu, ContextMenu, Menubar, NavigationMenu |
| Overlays | Popover, HoverCard, Tooltip, Drawer/Sheet, Command (+ CommandDialog) |
| Feedback | Progress, Toast + Toaster |
| Data | Table primitives + DataTable, Pagination (+ SimplePagination) |
| Selection | Toggle, ToggleGroup, Tabs |

Radix primitives back everything Radix covers; Combobox, Command, Calendar/
DatePicker, InputOTP, Drawer, Pagination, and Table are lightweight in-house
implementations (no cmdk, input-otp, date, or table deps).

Stories live under `stories/extras/` (Storybook titles `Extras/<Name>`);
smoke tests are colocated as `src/extras/*.test.tsx`.
