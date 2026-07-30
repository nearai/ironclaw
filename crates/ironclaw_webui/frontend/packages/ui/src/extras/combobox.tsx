/**
 * Combobox
 *
 * Searchable single-select: a SelectMenu-style trigger that opens a panel
 * with a filter input and a listbox of matching options. Hand-built on the
 * package's own popover/listbox conventions (no cmdk / downshift dep).
 *
 * ARIA follows the WAI combobox pattern: the search input is the combobox
 * (aria-expanded / aria-controls / aria-activedescendant); options are
 * role="option" rows navigated with ArrowUp/Down, chosen with Enter, and
 * dismissed with Escape.
 *
 * Usage
 *   <Combobox
 *     options={[{ value: "us-east", label: "US East" }, …]}
 *     value={region}
 *     onChange={setRegion}
 *     placeholder="Select region"
 *     aria-label="Region"
 *   />
 */
import React, { type ReactNode } from "react";
import { cn } from "../primitives/cn";
import { Icon } from "../icons/icon";
import { OVERLAY_SURFACE_CLASSES } from "../primitives/overlay";

export type ComboboxOption = {
  value: string;
  label?: string;
  disabled?: boolean;
};

type ComboboxProps = {
  options: ComboboxOption[];
  value?: string | null;
  onChange?: (value: string) => void;
  placeholder?: string;
  searchPlaceholder?: string;
  emptyMessage?: ReactNode;
  disabled?: boolean;
  "aria-label"?: string;
  className?: string;
};

let nextComboboxId = 0;

function labelOf(option: ComboboxOption | undefined | null): string {
  return option?.label ?? option?.value ?? "";
}

export function Combobox({
  options,
  value = null,
  onChange,
  placeholder = "Select…",
  searchPlaceholder = "Search…",
  emptyMessage = "No results",
  disabled = false,
  "aria-label": ariaLabel,
  className,
}: ComboboxProps) {
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const [activeIndex, setActiveIndex] = React.useState(0);
  const rootRef = React.useRef<HTMLDivElement | null>(null);
  const triggerRef = React.useRef<HTMLButtonElement | null>(null);
  const inputRef = React.useRef<HTMLInputElement | null>(null);
  const idRef = React.useRef("");
  if (!idRef.current) {
    nextComboboxId += 1;
    idRef.current = `v2-combobox-${nextComboboxId}`;
  }
  const listboxId = `${idRef.current}-listbox`;

  const normalizedQuery = query.trim().toLowerCase();
  const filtered = options.filter((option) =>
    labelOf(option).toLowerCase().includes(normalizedQuery)
  );
  const activeOption = filtered[activeIndex];
  const activeOptionId = activeOption
    ? `${idRef.current}-option-${activeOption.value}`
    : undefined;
  const selected = options.find((option) => option.value === value) ?? null;

  const close = React.useCallback((restoreFocus: boolean) => {
    setOpen(false);
    setQuery("");
    setActiveIndex(0);
    if (restoreFocus) triggerRef.current?.focus();
  }, []);

  React.useEffect(() => {
    if (!open) return;
    const handleMouseDown = (event: MouseEvent) => {
      if (!rootRef.current?.contains(event.target as Node | null)) close(false);
    };
    document.addEventListener("mousedown", handleMouseDown);
    return () => document.removeEventListener("mousedown", handleMouseDown);
  }, [open, close]);

  React.useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open]);

  const choose = (option: ComboboxOption | undefined) => {
    if (!option || option.disabled) return;
    onChange?.(option.value);
    close(true);
  };

  const moveActive = (direction: 1 | -1) => {
    if (filtered.length === 0) return;
    let index = activeIndex;
    for (let step = 0; step < filtered.length; step += 1) {
      index = (index + direction + filtered.length) % filtered.length;
      if (!filtered[index]?.disabled) break;
    }
    setActiveIndex(index);
  };

  const handleInputKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      moveActive(event.key === "ArrowDown" ? 1 : -1);
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      choose(filtered[activeIndex]);
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      close(true);
      return;
    }
    if (event.key === "Tab") close(false);
  };

  return (
    <div
      ref={rootRef}
      className={cn("relative inline-block min-w-[12rem] text-left font-sans text-ui", className)}
    >
      <button
        ref={triggerRef}
        type="button"
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
        onClick={() => (open ? close(false) : setOpen(true))}
        onKeyDown={(event) => {
          if (!open && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
            event.preventDefault();
            setOpen(true);
          }
        }}
        className={cn(
          "inline-flex h-9 w-full items-center justify-between gap-2 rounded-[10px] border px-2.5",
          "border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] text-left transition-colors",
          selected ? "text-[var(--v2-text-strong)]" : "text-[var(--v2-text-faint)]",
          "hover:bg-[var(--v2-surface-soft)]",
          "hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",
          "active:bg-[var(--v2-surface-muted)]",
          "focus-visible:outline-none focus-visible:ring-2",
          "focus-visible:ring-[var(--v2-focus-ring)]",
          "disabled:cursor-not-allowed disabled:opacity-50",
          "disabled:hover:border-[var(--v2-panel-border)] disabled:hover:bg-[var(--v2-input-bg)]"
        )}
      >
        <span className="truncate">{selected ? labelOf(selected) : placeholder}</span>
        <Icon
          name="chevron"
          className={cn(
            "h-3.5 w-3.5 shrink-0 text-[var(--v2-text-faint)] transition-transform",
            open && "rotate-180"
          )}
        />
      </button>

      {open && (
        <div
          className={cn(
            OVERLAY_SURFACE_CLASSES,
            "absolute left-0 top-[calc(100%+0.35rem)] w-full min-w-full"
          )}
        >
          <div className="flex items-center gap-2 border-b border-[var(--v2-panel-border)] px-2.5 pb-2 pt-1.5">
            <Icon name="search" className="h-3.5 w-3.5 shrink-0 text-[var(--v2-text-faint)]" />
            <input
              ref={inputRef}
              role="combobox"
              aria-expanded="true"
              aria-controls={listboxId}
              aria-activedescendant={activeOptionId}
              aria-autocomplete="list"
              aria-label={ariaLabel}
              value={query}
              placeholder={searchPlaceholder}
              onChange={(event) => {
                setQuery(event.target.value);
                setActiveIndex(0);
              }}
              onKeyDown={handleInputKeyDown}
              className={cn(
                "w-full rounded-[4px] bg-transparent text-ui text-[var(--v2-text-strong)] outline-none",
                "focus-visible:ring-2 focus-visible:ring-[var(--v2-focus-ring)]",
                "placeholder:text-[var(--v2-text-faint)]"
              )}
            />
          </div>
          <div id={listboxId} role="listbox" className="max-h-56 overflow-y-auto pt-1">
            {filtered.length === 0 && (
              <div className="px-2.5 py-4 text-center text-ui-sm text-[var(--v2-text-faint)]">
                {emptyMessage}
              </div>
            )}
            {filtered.map((option, index) => {
              const isSelected = option.value === value;
              const isActive = index === activeIndex;
              return (
                <button
                  key={option.value}
                  id={`${idRef.current}-option-${option.value}`}
                  type="button"
                  role="option"
                  tabIndex={-1}
                  aria-selected={isSelected}
                  aria-disabled={option.disabled || undefined}
                  disabled={option.disabled}
                  onMouseEnter={() => !option.disabled && setActiveIndex(index)}
                  onClick={() => choose(option)}
                  className={cn(
                    "flex w-full items-center justify-between gap-3 rounded-[7px] px-2.5 py-2",
                    "text-left text-ui text-[var(--v2-text)] transition-colors",
                    "disabled:cursor-not-allowed disabled:opacity-50",
                    isActive
                      ? "bg-[var(--v2-surface-muted)] text-[var(--v2-text-strong)]"
                      : isSelected && "bg-[var(--v2-accent-soft)] text-[var(--v2-text-strong)]"
                  )}
                >
                  <span className="truncate">{labelOf(option)}</span>
                  {isSelected && (
                    <Icon name="check" className="h-3.5 w-3.5 shrink-0 text-[var(--v2-accent-text)]" />
                  )}
                </button>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
