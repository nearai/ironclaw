/**
 * SelectMenu
 *
 * Custom select built on `@radix-ui/react-select` (shadcn Select pattern)
 * while preserving the existing public API: value / options / onChange /
 * prefix / tones / align / placeholder / aria labels.
 */
import * as Select from "@radix-ui/react-select";
import type { ReactNode } from "react";
import { cn } from "./cn";
import { Icon } from "./icons";

const toneDotClasses = {
  neutral: "bg-[var(--v2-text-faint)]",
  positive: "bg-[var(--v2-positive-text)]",
  warning: "bg-[var(--v2-warning-text)]",
  danger: "bg-[var(--v2-danger-text)]",
  info: "bg-[var(--v2-info-text)]",
  accent: "bg-[var(--v2-accent-text)]",
} as const;

export type SelectMenuTone = keyof typeof toneDotClasses;
export type SelectMenuAlign = "left" | "right";
export type SelectMenuOption = {
  value: string;
  label?: string;
  disabled?: boolean;
  /** Known tones map to dots; unknown values fall back to neutral. */
  tone?: SelectMenuTone | string;
};

function normalizeTone(tone?: string | null): SelectMenuTone | null {
  if (!tone) return null;
  return Object.prototype.hasOwnProperty.call(toneDotClasses, tone)
    ? (tone as SelectMenuTone)
    : "neutral";
}

function optionLabel(option: SelectMenuOption | null | undefined, fallback = "") {
  return option?.label ?? option?.value ?? fallback;
}

function ToneDot({ tone }: { tone?: string | null }) {
  const normalizedTone = normalizeTone(tone);
  if (!normalizedTone) return null;
  return (
    <span
      aria-hidden="true"
      className={cn("h-1.5 w-1.5 shrink-0 rounded-full", toneDotClasses[normalizedTone])}
    />
  );
}

function safeRootProps(props: Record<string, unknown>) {
  return Object.fromEntries(
    Object.entries(props).filter(
      ([key]) =>
        key === "id" ||
        key === "title" ||
        key.startsWith("data-") ||
        key.startsWith("aria-")
    )
  );
}

type SelectMenuProps = {
  value?: string;
  options?: SelectMenuOption[];
  onChange?: (value: string) => void;
  disabled?: boolean;
  ariaLabel?: string;
  "aria-label"?: string;
  "aria-labelledby"?: string;
  className?: string;
  buttonClassName?: string;
  menuClassName?: string;
  optionClassName?: string;
  align?: SelectMenuAlign;
  placeholder?: string;
  prefix?: ReactNode;
  [key: string]: unknown;
};

export function SelectMenu({
  value,
  options = [],
  onChange = (_value) => {},
  disabled = false,
  ariaLabel = undefined,
  "aria-label": ariaLabelProp = undefined,
  "aria-labelledby": ariaLabelledBy = undefined,
  className = "",
  buttonClassName = "",
  menuClassName = "",
  optionClassName = "",
  align = "right",
  placeholder = "",
  prefix = "",
  ...rest
}: SelectMenuProps) {
  const selectedOption = options.find((option) => option.value === value) ?? null;
  const selectedLabel = optionLabel(selectedOption, placeholder);
  const effectiveAriaLabel = ariaLabel || ariaLabelProp;
  const hasEnabledOption = options.some((option) => !option.disabled);
  const interactionDisabled = disabled || !hasEnabledOption;
  const rootPassthroughProps = safeRootProps(rest as Record<string, unknown>);
  const contentAlign = align === "left" ? "start" : "end";

  return (
    <div
      className={cn("relative inline-block min-w-[9.5rem] text-left", className)}
      {...rootPassthroughProps}
    >
      <Select.Root
        value={value || undefined}
        onValueChange={onChange}
        disabled={interactionDisabled}
      >
        <Select.Trigger
          aria-label={effectiveAriaLabel}
          aria-labelledby={ariaLabelledBy}
          className={cn(
            "group inline-flex h-8 w-full items-center justify-between gap-2 rounded-[8px] border",
            "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-2.5",
            "font-mono text-[11px] text-[var(--v2-text-strong)] shadow-none",
            "transition-[background,border-color,color,box-shadow] duration-[var(--v2-duration-fast)]",
            "ease-[var(--v2-ease-standard)] hover:bg-[var(--v2-surface-muted)]",
            "focus-visible:outline-none focus-visible:ring-2",
            "focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_32%,transparent)]",
            "disabled:cursor-not-allowed disabled:opacity-60",
            "data-[placeholder]:text-[var(--v2-text-faint)]",
            buttonClassName
          )}
        >
          <span className="flex min-w-0 items-center gap-2">
            {prefix ? (
              <span className="shrink-0 text-[var(--v2-text-faint)]">{prefix}</span>
            ) : null}
            <ToneDot tone={selectedOption?.tone} />
            {/* Render the label ourselves so SSR / closed-portal states still
                show the selection; Select.Value alone is empty until items
                mount in the portal. */}
            <Select.Value placeholder={placeholder}>
              <span className="truncate">{selectedLabel || placeholder}</span>
            </Select.Value>
          </span>
          <Select.Icon asChild>
            <Icon
              name="chevron"
              className="h-3.5 w-3.5 shrink-0 text-[var(--v2-text-faint)] transition-transform group-data-[state=open]:rotate-180"
            />
          </Select.Icon>
        </Select.Trigger>

        <Select.Portal>
          <Select.Content
            position="popper"
            sideOffset={6}
            align={contentAlign}
            className={cn(
              "z-30 min-w-[var(--radix-select-trigger-width)] overflow-hidden rounded-[10px]",
              "border border-[color-mix(in_srgb,var(--v2-text-strong)_16%,var(--v2-panel-border))]",
              "bg-[color-mix(in_srgb,var(--v2-canvas-strong)_92%,var(--v2-surface))] p-1",
              "shadow-[var(--v2-shadow-menu)]",
              "data-[state=open]:animate-in data-[state=closed]:animate-out",
              menuClassName
            )}
          >
            <Select.Viewport className="max-h-64 p-0">
              {options.map((option) => {
                return (
                  <Select.Item
                    key={option.value}
                    value={option.value}
                    disabled={option.disabled}
                    className={cn(
                      "relative flex w-full cursor-default items-center justify-between gap-3 rounded-[7px] px-2.5 py-2",
                      "text-left font-mono text-[11px] text-[var(--v2-text)] outline-none select-none",
                      "transition-colors duration-[var(--v2-duration-fast)]",
                      "data-[highlighted]:bg-[var(--v2-surface-muted)] data-[highlighted]:text-[var(--v2-text-strong)]",
                      "data-[state=checked]:bg-[var(--v2-accent-soft)] data-[state=checked]:text-[var(--v2-text-strong)]",
                      "data-[disabled]:cursor-not-allowed data-[disabled]:opacity-50",
                      optionClassName
                    )}
                  >
                    <span className="flex min-w-0 items-center gap-2">
                      <ToneDot tone={option.tone} />
                      <Select.ItemText>{optionLabel(option)}</Select.ItemText>
                    </span>
                    <Select.ItemIndicator>
                      <Icon
                        name="check"
                        className="h-3.5 w-3.5 shrink-0 text-[var(--v2-accent-text)]"
                      />
                    </Select.ItemIndicator>
                  </Select.Item>
                );
              })}
            </Select.Viewport>
          </Select.Content>
        </Select.Portal>
      </Select.Root>
    </div>
  );
}
