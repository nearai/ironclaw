/**
 * Tabs
 *
 * Underline tab row built on `@radix-ui/react-tabs` for keyboard roving +
 * aria, with a token-driven CSS underline entrance (`.v2-tab-underline`
 * in tokens.css). CSS-only motion keeps the framer-motion runtime out of
 * every route's initial bundle (see scripts/check-bundle-budgets.ts).
 * Public API unchanged:
 *   tabs / value / onChange / ariaLabel / bordered / className
 */
import * as TabsPrimitive from "@radix-ui/react-tabs";
import { cn } from "./cn";

export type TabItem = {
  value: string;
  label: string;
  count?: number;
};

type TabsProps = {
  tabs?: TabItem[];
  value?: string;
  onChange?: (value: string) => void;
  ariaLabel?: string;
  bordered?: boolean;
  className?: string;
};

export function Tabs({
  tabs = [],
  value,
  onChange = (_value) => {},
  ariaLabel = undefined,
  bordered = true,
  className = "",
}: TabsProps) {
  return (
    <TabsPrimitive.Root value={value} onValueChange={onChange}>
      <TabsPrimitive.List
        aria-label={ariaLabel}
        className={cn(
          "flex max-w-full items-stretch gap-1",
          bordered && "border-b border-[var(--v2-panel-border)]",
          className
        )}
      >
        {tabs.map((tab) => {
          const selected = tab.value === value;
          return (
            <TabsPrimitive.Trigger
              key={tab.value}
              value={tab.value}
              className={cn(
                "relative -mb-px inline-flex min-h-[calc(var(--v2-control-h-md)+var(--v2-control-px-sm))] shrink-0 items-center gap-1.5",
                "border-b-2 border-transparent px-[var(--v2-control-px-sm)] text-[13px] font-medium",
                "transition-colors focus-visible:outline-none focus-visible:ring-2",
                "focus-visible:ring-inset focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_40%,transparent)]",
                "data-[state=active]:text-[var(--v2-text-strong)]",
                "data-[state=inactive]:text-[var(--v2-text-muted)] data-[state=inactive]:hover:text-[var(--v2-text-strong)]"
              )}
            >
              {tab.label}
              {tab.count != null && (
                <span
                  className={cn(
                    "inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-full px-1",
                    "font-mono text-[10px] tabular-nums leading-none",
                    selected
                      ? "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
                      : "bg-[var(--v2-surface-muted)] text-[var(--v2-text-muted)]"
                  )}
                >
                  {tab.count}
                </span>
              )}
              {selected && (
                <span
                  aria-hidden="true"
                  className="v2-tab-underline absolute inset-x-0 -bottom-[2px] h-[2px] bg-[var(--v2-accent)]"
                />
              )}
            </TabsPrimitive.Trigger>
          );
        })}
      </TabsPrimitive.List>
    </TabsPrimitive.Root>
  );
}
