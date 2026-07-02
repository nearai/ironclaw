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
import { useSearchParams } from "react-router";
import { React, html } from "../../lib/html.js";
import { cn } from "../../utils/cn.js";
import { Icon } from "../../design-system/icons.js";
import {
  ColorsSection,
  MotionSection,
  RadiiSection,
  SpacingSection,
  TypographySection,
  ZIndexSection,
} from "./components/token-sections.js";
import {
  BadgeSection,
  ButtonSection,
  CardSection,
  IconSection,
  InputSection,
  ModalSection,
  PrimitivesSection,
} from "./components/component-sections.js";

const THEME_STORAGE_KEY = "ironclaw:v2-theme";

/* Theme hook with a *synchronous* DOM write on toggle, so token
   sections reading getComputedStyle during render see the new theme
   in the same pass (the shared useInterfaceTheme applies the theme in
   an effect, which runs after children have already rendered). */
function usePlaygroundTheme() {
  const [theme, setTheme] = React.useState(() =>
    document.documentElement.dataset.theme === "dark" ? "dark" : "light"
  );
  const toggleTheme = React.useCallback(() => {
    setTheme((current) => {
      const next = current === "dark" ? "light" : "dark";
      document.documentElement.dataset.theme = next;
      try {
        window.localStorage.setItem(THEME_STORAGE_KEY, next);
      } catch (_) {}
      return next;
    });
  }, []);
  return { theme, toggleTheme };
}

const SECTIONS = [
  {
    label: "Tokens",
    items: [
      { id: "colors", name: "Colors", icon: "spark", render: ColorsSection, blurb: "Semantic color roles — every value is a --v2-* custom property that resolves per theme." },
      { id: "typography", name: "Typography", icon: "file", render: TypographySection, blurb: "Type scale, tracking, and weights derived from what components render." },
      { id: "spacing", name: "Spacing", icon: "list", render: SpacingSection, blurb: "The 4px grid. Pick the named step that matches the relationship, not a raw value." },
      { id: "radii", name: "Radii & Shadows", icon: "layers", render: RadiiSection, blurb: "Corner radii by control size, plus the two elevation shadows." },
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
      { id: "modal", name: "Modal", icon: "plus", render: ModalSection, blurb: "The only dialog primitive. Escape + backdrop close, scroll lock, modal layer." },
      { id: "icons", name: "Icons", icon: "settings", render: IconSection, blurb: "Inline 24px stroke icons. Add new glyphs to icons.js, never inline new SVG." },
      { id: "primitives", name: "Primitives", icon: "shield", render: PrimitivesSection, blurb: "Higher-level composites: StatCard, FlowList, EmptyPanel, SectionHeader, SubLabel." },
    ],
  },
];

const ALL_ITEMS = SECTIONS.flatMap((section) => section.items);
const DEFAULT_ITEM = "colors";

function NavItem({ active, onClick, icon, children }) {
  return html`
    <button
      type="button"
      onClick=${onClick}
      className=${cn(
        "flex items-center gap-2 rounded-[10px] px-2.5 py-1.5 text-left text-[0.8125rem] font-medium",
        active
          ? "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
          : "text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]"
      )}
    >
      <${Icon} name=${icon} className="h-3.5 w-3.5 opacity-80" />
      ${children}
    </button>
  `;
}

export function PlaygroundPage() {
  const { theme, toggleTheme } = usePlaygroundTheme();
  const [params, setParams] = useSearchParams();
  const itemId = params.get("item") || DEFAULT_ITEM;
  const item = ALL_ITEMS.find((entry) => entry.id === itemId) || ALL_ITEMS[0];

  const select = React.useCallback(
    (id) => setParams({ item: id }, { replace: true }),
    [setParams]
  );

  const Body = item.render;

  return html`
    <div className="flex min-h-[100dvh] bg-[var(--v2-canvas)] text-[var(--v2-text)]">
      <!-- Left rail -->
      <aside
        className="sticky top-0 flex h-[100dvh] w-60 shrink-0 flex-col overflow-y-auto border-r border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-4 pb-8 pt-5"
      >
        <div className="mb-6 flex items-center gap-2.5 px-1">
          <span
            className="grid h-8 w-8 place-items-center rounded-[10px] border border-[var(--v2-accent-soft)] bg-[var(--v2-accent-soft)]"
          >
            <span className="font-mono text-sm font-semibold text-[var(--v2-accent-text)]">IC</span>
          </span>
          <div className="flex flex-col">
            <span className="text-[0.8125rem] font-semibold leading-tight text-[var(--v2-text-strong)]">
              IronClaw UI
            </span>
            <span className="text-[0.625rem] leading-tight text-[var(--v2-text-muted)]">
              design-system playground
            </span>
          </div>
        </div>

        ${SECTIONS.map(
          (section) => html`
            <div key=${section.label} className="mb-5">
              <p
                className="mb-1.5 px-2 font-mono text-[0.625rem] font-semibold uppercase tracking-[0.14em] text-[var(--v2-text-faint)]"
              >
                ${section.label}
              </p>
              <div className="flex flex-col gap-0.5">
                ${section.items.map(
                  (entry) => html`
                    <${NavItem}
                      key=${entry.id}
                      active=${entry.id === item.id}
                      icon=${entry.icon}
                      onClick=${() => select(entry.id)}
                    >
                      ${entry.name}
                    <//>
                  `
                )}
              </div>
            </div>
          `
        )}

        <div className="mt-auto px-1 pt-4">
          <p className="text-[0.625rem] leading-4 text-[var(--v2-text-faint)]">
            Rules: crates/ironclaw_webui_v2/DESIGN_SYSTEM.md
          </p>
        </div>
      </aside>

      <!-- Canvas -->
      <main className="min-w-0 flex-1 overflow-y-auto px-6 pb-16 pt-7 lg:px-10">
        <header className="mb-7 flex items-start justify-between gap-6">
          <div>
            <h1 className="text-xl font-semibold tracking-[-0.02em] text-[var(--v2-text-strong)]">
              ${item.name}
            </h1>
            <p className="mt-1 max-w-[60ch] text-[0.8125rem] leading-relaxed text-[var(--v2-text-muted)]">
              ${item.blurb}
            </p>
          </div>
          <button
            type="button"
            onClick=${toggleTheme}
            title="Toggle theme"
            className="flex shrink-0 items-center gap-2 rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] px-3 py-1.5 text-[0.75rem] font-medium text-[var(--v2-text-muted)] hover:text-[var(--v2-text-strong)]"
          >
            <${Icon} name=${theme === "dark" ? "moon" : "sun"} className="h-3.5 w-3.5" />
            ${theme === "dark" ? "Dark" : "Light"}
          </button>
        </header>

        <${Body} key=${item.id + theme} theme=${theme} />
      </main>
    </div>
  `;
}
