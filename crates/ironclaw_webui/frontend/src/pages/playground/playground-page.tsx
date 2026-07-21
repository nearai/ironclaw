/**
 * /playground — the design-system workbench.
 *
 * Renders every token group and every design-system component with
 * its variants/states, on a full-bleed canvas outside the app chrome
 * (modeled on the Rips /ui/playground). Selection syncs to ?item= so
 * a specific token page or component is shareable. Purely static:
 * no API calls, no auth requirement — it is a reference surface for
 * humans and agents, documented in DESIGN_SYSTEM.md.
 */
import { useCallback, useState } from "react";
import { useSearchParams } from "react-router";
import { cn } from "@ironclaw/design-system";
import { Icon } from "@ironclaw/design-system";
import {
  ColorsSection,
  MotionSection,
  RadiiSection,
  SpacingSection,
  TypographySection,
  ZIndexSection,
} from "./components/token-sections";
import {
  AvatarSkeletonSection,
  BadgeSection,
  ButtonSection,
  CardSection,
  CheckboxSwitchSection,
  DropdownPopoverSection,
  IconSection,
  InputSection,
  ModalSection,
  PrimitivesSection,
  SelectMenuSection,
  TabsSection,
  TooltipSection,
} from "./components/component-sections";
import { STORYBOOK_URL, USAGE_DOCS, UsageBlock } from "./components/usage-docs";

const THEME_STORAGE_KEY = "ironclaw:v2-theme";

/* Theme hook with a *synchronous* DOM write on toggle, so token
   sections reading getComputedStyle during render see the new theme
   in the same pass (the shared useInterfaceTheme applies the theme in
   an effect, which runs after children have already rendered). */
function usePlaygroundTheme() {
  const [theme, setTheme] = useState(() =>
    document.documentElement.dataset.theme === "dark" ? "dark" : "light"
  );
  const toggleTheme = useCallback(() => {
    // Side effects live in the handler (not the state updater, which
    // must stay pure): the DOM write happens before the re-render so
    // getComputedStyle reads the new theme in the same pass.
    const next = theme === "dark" ? "light" : "dark";
    document.documentElement.dataset.theme = next;
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, next);
    } catch (_) { /* private mode */ }
    setTheme(next);
  }, [theme]);
  return { theme, toggleTheme };
}

const SECTIONS = [
  {
    label: "Tokens",
    items: [
      { id: "colors", name: "Colors", icon: "spark", render: ColorsSection, blurb: "Semantic color roles — every value is a --v2-* custom property that resolves per theme." },
      { id: "typography", name: "Typography", icon: "file", render: TypographySection, blurb: "Type scale, tracking, and weights derived from what components render." },
      { id: "spacing", name: "Spacing", icon: "list", render: SpacingSection, blurb: "The 4px grid. Pick the named step that matches the relationship, not a raw value." },
      { id: "radii", name: "Radii & Shadows", icon: "layers", render: RadiiSection, blurb: "Corner radii by control size, plus the restrained elevation scale — borders separate, shadows only lift." },
      { id: "motion", name: "Motion", icon: "pulse", render: MotionSection, blurb: "The restrained-motion system: duration + easing tokens, entrances, and the ambient work indicators." },
      { id: "z-index", name: "Z-index", icon: "logs", render: ZIndexSection, blurb: "The five-layer ladder. Every overlay picks a layer, never a number." },
    ],
  },
  {
    label: "Components",
    items: [
      { id: "button", name: "Button", icon: "bolt", render: ButtonSection, blurb: "Five variants, five sizes. Primary is for the single main action on a surface." },
      { id: "badge", name: "Badge", icon: "check", render: BadgeSection, blurb: "Status chips. Tone communicates state; label carries the translated copy." },
      { id: "card", name: "Card", icon: "folder", render: CardSection, blurb: "The panel surface. Compose with CardHeader / CardBody / CardFooter / CardLabel." },
      { id: "input", name: "Inputs", icon: "terminal", render: InputSection, blurb: "Input, Textarea, Select, Label, and the FormField wrapper with hint/error slots." },
      { id: "select-menu", name: "SelectMenu", icon: "chevron", render: SelectMenuSection, blurb: "Listbox-backed custom select: option tones, keyboard support, and an inline prefix label for dense toolbars." },
      { id: "tabs", name: "Tabs", icon: "list", render: TabsSection, blurb: "Underline tab row for single-select filters — sized by the control tokens so it aligns with adjacent toolbar controls." },
      { id: "modal", name: "Modal", icon: "plus", render: ModalSection, blurb: "Radix Dialog + IronClaw motion. Escape + backdrop close, focus trap, modal layer." },
      { id: "tooltip", name: "Tooltip", icon: "spark", render: TooltipSection, blurb: "Radix Tooltip for short hover/focus hints." },
      { id: "controls", name: "Checkbox / Switch / Radio / Slider", icon: "check", render: CheckboxSwitchSection, blurb: "Radix form controls styled with --v2-* tokens." },
      { id: "menus", name: "Dropdown / Popover", icon: "more", render: DropdownPopoverSection, blurb: "Action menus (DropdownMenu) and anchored content (Popover)." },
      { id: "display", name: "Avatar / Skeleton / Scroll", icon: "layers", render: AvatarSkeletonSection, blurb: "Avatar, Skeleton, Separator, and ScrollArea." },
      { id: "icons", name: "Icons", icon: "settings", render: IconSection, blurb: "Inline 24px stroke icons. Add new glyphs to icons.tsx, never inline new SVG." },
      { id: "primitives", name: "Primitives", icon: "shield", render: PrimitivesSection, blurb: "Higher-level composites: StatCard, FlowList, EmptyPanel, SectionHeader, SubLabel." },
    ],
  },
];

const ALL_ITEMS = SECTIONS.flatMap((section) => section.items);
const DEFAULT_ITEM = "colors";

function NavItem({ active, onClick, icon, children }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "flex items-center gap-2 rounded-[10px] px-2.5 py-1.5 text-left text-[0.8125rem] font-medium",
        active
          ? "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
          : "text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]"
      )}
    >
      <Icon name={icon} className="h-3.5 w-3.5 opacity-80" />
      {children}
    </button>
  );
}

export function PlaygroundPage() {
  const { theme, toggleTheme } = usePlaygroundTheme();
  const [params, setParams] = useSearchParams();
  const itemId = params.get("item") || DEFAULT_ITEM;
  const item = ALL_ITEMS.find((entry) => entry.id === itemId) || ALL_ITEMS[0];

  const select = useCallback(
    (id) => setParams({ item: id }, { replace: true }),
    [setParams]
  );

  const Body = item.render;

  return (
    <div className="flex min-h-[100dvh] bg-[var(--v2-canvas)] text-[var(--v2-text)]">
      {/* Left rail */}
      <aside
        className="sticky top-0 flex h-[100dvh] w-60 shrink-0 flex-col overflow-y-auto border-r border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-4 pb-8 pt-5"
      >
        <div className="mb-6 flex items-center gap-2.5 px-1">
          <span
            className="grid h-8 w-8 place-items-center rounded-[10px] border border-[var(--v2-accent-soft)] bg-[var(--v2-accent-soft)]"
          >
            <span className="font-mono text-sm font-medium text-[var(--v2-accent-text)]">IC</span>
          </span>
          <div className="flex flex-col">
            <span className="text-[0.8125rem] font-medium leading-tight text-[var(--v2-text-strong)]">
              IronClaw UI
            </span>
            <span className="text-[0.625rem] leading-tight text-[var(--v2-text-muted)]">
              design-system playground
            </span>
          </div>
        </div>

        {SECTIONS.map((section) => (
          <div key={section.label} className="mb-5">
            <p className="v2-tag-face mb-1.5 px-2 text-[0.625rem] text-[var(--v2-text-faint)]">
              {section.label}
            </p>
            <div className="flex flex-col gap-0.5">
              {section.items.map((entry) => (
                <NavItem
                  key={entry.id}
                  active={entry.id === item.id}
                  icon={entry.icon}
                  onClick={() => select(entry.id)}
                >
                  {entry.name}
                </NavItem>
              ))}
            </div>
          </div>
        ))}

        {/* Footer: Storybook (the canonical hosted spec) + the written
            rules. Quiet card rows with hover tooltips. */}
        <a
          href={STORYBOOK_URL}
          target="_blank"
          rel="noreferrer"
          title="Full spec: every variant, props controls, a11y checks, and story source"
          className="mt-auto mb-2 flex items-center gap-2.5 rounded-[var(--v2-radius-md)] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2.5 hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]"
        >
          <Icon name="layers" className="h-4 w-4 shrink-0 text-[var(--v2-accent-text)]" />
          <div className="flex min-w-0 flex-col gap-0.5">
            <span className="text-[0.75rem] font-medium leading-tight text-[var(--v2-text-strong)]">
              Storybook
            </span>
            <span className="truncate font-mono text-[0.625rem] leading-tight text-[var(--v2-text-faint)]">
              hosted component spec
            </span>
          </div>
        </a>
        <div
          className="flex items-center gap-2.5 rounded-[var(--v2-radius-md)] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2.5"
          title="crates/ironclaw_webui/DESIGN_SYSTEM.md"
        >
          <Icon
            name="bookOpen"
            className="h-4 w-4 shrink-0 text-[var(--v2-text-muted)]"
          />
          <div className="flex min-w-0 flex-col gap-0.5">
            <span className="text-[0.75rem] font-medium leading-tight text-[var(--v2-text-strong)]">
              Design rules
            </span>
            <span className="truncate font-mono text-[0.625rem] leading-tight text-[var(--v2-text-faint)]">
              DESIGN_SYSTEM.md
            </span>
          </div>
        </div>
      </aside>

      {/* Canvas */}
      <main className="min-w-0 flex-1 overflow-y-auto px-6 pb-16 pt-7 lg:px-10">
        <header className="mb-7 flex items-start justify-between gap-6">
          <div>
            <h1 className="text-xl font-medium tracking-[-0.02em] text-[var(--v2-text-strong)]">
              {item.name}
            </h1>
            <p className="mt-1 max-w-[60ch] text-[0.8125rem] leading-relaxed text-[var(--v2-text-muted)]">
              {item.blurb}
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {USAGE_DOCS[item.id] && (
              <a
                href={`${STORYBOOK_URL}${USAGE_DOCS[item.id].storybook}`}
                target="_blank"
                rel="noreferrer"
                title="Open the full spec in Storybook"
                className="flex items-center gap-2 rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] px-3 py-1.5 text-[0.75rem] font-medium text-[var(--v2-text-muted)] hover:text-[var(--v2-text-strong)]"
              >
                <Icon name="bookOpen" className="h-3.5 w-3.5" />
                Storybook
              </a>
            )}
            <button
              type="button"
              onClick={toggleTheme}
              title="Toggle theme"
              className="flex items-center gap-2 rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] px-3 py-1.5 text-[0.75rem] font-medium text-[var(--v2-text-muted)] hover:text-[var(--v2-text-strong)]"
            >
              <Icon name={theme === "dark" ? "moon" : "sun"} className="h-3.5 w-3.5" />
              {theme === "dark" ? "Dark" : "Light"}
            </button>
          </div>
        </header>

        <UsageBlock doc={USAGE_DOCS[item.id]} />

        <Body key={item.id + theme} theme={theme} />
      </main>
    </div>
  );
}
