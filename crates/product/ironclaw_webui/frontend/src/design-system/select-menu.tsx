import React, {
  type AriaAttributes,
  type ComponentPropsWithoutRef,
  type FocusEvent,
  type KeyboardEvent,
  type RefObject,
} from "react";
import { cn } from "../utils/cn";
import { Icon } from "./icons";
import type { DataAttributes } from "./types";

type OpenSelectMenuEntry = {
  close: () => void;
  rootRef: RefObject<HTMLDivElement | null>;
};

let nextSelectMenuId = 0;
const openSelectMenuEntries = new Set<OpenSelectMenuEntry>();
let sharedDocumentMouseDownListener: ((event: MouseEvent) => void) | null = null;

const toneDotClasses = {
  neutral: "bg-[var(--v2-text-faint)]",
  positive: "bg-[var(--v2-positive-text)]",
  warning: "bg-[var(--v2-warning-text)]",
  danger: "bg-[var(--v2-danger-text)]",
  info: "bg-[var(--v2-info-text)]",
  accent: "bg-[var(--v2-accent-text)]",
};

const alignClasses = {
  left: "left-0",
  right: "right-0",
};

const sizeClasses = {
  md: {
    root: "min-w-[9.5rem] text-ui",
    button: "h-8 px-2.5",
    option: "px-2.5 py-2",
  },
  sm: {
    root: "min-w-[7.5rem] text-xs",
    button: "h-8 px-2",
    option: "px-2 py-1.5 text-xs",
  },
};

export type SelectMenuTone = keyof typeof toneDotClasses;
export type SelectMenuAlign = keyof typeof alignClasses;
export type SelectMenuSize = keyof typeof sizeClasses;

export type SelectMenuOption = {
  disabled?: boolean;
  label?: string;
  tone?: SelectMenuTone;
  value: string;
};

type SelectMenuRootProps = Pick<
  ComponentPropsWithoutRef<"div">,
  "id" | "title"
> &
  AriaAttributes &
  DataAttributes;

export type SelectMenuProps = SelectMenuRootProps & {
  align?: SelectMenuAlign;
  ariaLabel?: string;
  buttonClassName?: string;
  className?: string;
  disabled?: boolean;
  menuClassName?: string;
  onChange?: (value: string) => void;
  optionClassName?: string;
  options?: readonly SelectMenuOption[];
  placeholder?: string;
  searchAriaLabel?: string;
  searchable?: boolean;
  searchPlaceholder?: string;
  size?: SelectMenuSize;
  value: string;
};

function createSelectMenuId() {
  nextSelectMenuId += 1;
  return `v2-select-menu-${nextSelectMenuId}`;
}

function removeStaleOpenSelectMenuEntries(): void {
  for (const entry of Array.from(openSelectMenuEntries)) {
    const root = entry.rootRef.current;
    if (!root || root.isConnected === false) openSelectMenuEntries.delete(entry);
  }
}

function handleSharedDocumentMouseDown(event: MouseEvent): void {
  removeStaleOpenSelectMenuEntries();
  for (const entry of Array.from(openSelectMenuEntries)) {
    if (entry.rootRef.current?.contains(event.target as Node | null)) continue;
    entry.close();
  }
  syncSharedDocumentListener();
}

function syncSharedDocumentListener(): void {
  if (typeof document === "undefined") return;
  if (openSelectMenuEntries.size > 0 && !sharedDocumentMouseDownListener) {
    sharedDocumentMouseDownListener = handleSharedDocumentMouseDown;
    document.addEventListener("mousedown", sharedDocumentMouseDownListener);
    return;
  }
  if (openSelectMenuEntries.size === 0 && sharedDocumentMouseDownListener) {
    document.removeEventListener("mousedown", sharedDocumentMouseDownListener);
    sharedDocumentMouseDownListener = null;
  }
}

function registerOpenSelectMenu(entry: OpenSelectMenuEntry): () => void {
  removeStaleOpenSelectMenuEntries();
  openSelectMenuEntries.add(entry);
  syncSharedDocumentListener();
  return () => {
    openSelectMenuEntries.delete(entry);
    syncSharedDocumentListener();
  };
}

function firstEnabledIndex(options: readonly SelectMenuOption[]): number {
  return options.findIndex((option) => !option.disabled);
}

function selectedOptionIndex(
  options: readonly SelectMenuOption[],
  value: string,
): number {
  const index = options.findIndex((option) => option.value === value);
  return index >= 0 ? index : firstEnabledIndex(options);
}

function nextEnabledIndex(
  options: readonly SelectMenuOption[],
  currentIndex: number,
  direction: 1 | -1,
): number {
  const fallbackIndex = firstEnabledIndex(options);
  if (fallbackIndex < 0) return -1;
  const start =
    currentIndex >= 0 ? currentIndex : direction > 0 ? -1 : options.length;
  for (let step = 1; step <= options.length; step += 1) {
    const index = (start + direction * step + options.length) % options.length;
    if (!options[index]?.disabled) return index;
  }
  return fallbackIndex;
}

function edgeEnabledIndex(
  options: readonly SelectMenuOption[],
  direction: 1 | -1,
): number {
  if (!options.length) return -1;
  const start = direction > 0 ? 0 : options.length - 1;
  const end = direction > 0 ? options.length : -1;
  for (let index = start; index !== end; index += direction) {
    if (!options[index]?.disabled) return index;
  }
  return -1;
}

function optionLabel(
  option: SelectMenuOption | null | undefined,
  fallback = "",
): string {
  return option?.label ?? option?.value ?? fallback;
}

function normalizeTone(
  tone: SelectMenuTone | null | undefined,
): SelectMenuTone | null {
  if (!tone) return null;
  return Object.prototype.hasOwnProperty.call(toneDotClasses, tone) ? tone : "neutral";
}

function normalizeAlign(align: SelectMenuAlign): SelectMenuAlign {
  return Object.prototype.hasOwnProperty.call(alignClasses, align) ? align : "right";
}

function optionsIdentity(options: readonly SelectMenuOption[]): string {
  return options
    .map((option) => `${String(option.value)}:${option.disabled ? "disabled" : "enabled"}`)
    .join("\u001f");
}

function safeRootProps(props: SelectMenuRootProps): SelectMenuRootProps {
  return Object.fromEntries(
    Object.entries(props).filter(
      ([key]) =>
        key === "id" ||
        key === "title" ||
        key.startsWith("data-") ||
        key.startsWith("aria-")
    )
  ) as SelectMenuRootProps;
}

function ToneDot({ tone }: { tone?: SelectMenuTone | null }) {
  const normalizedTone = normalizeTone(tone);
  if (!normalizedTone) return null;
  return (
    <span
      aria-hidden="true"
      className={cn(
        "h-1.5 w-1.5 shrink-0 rounded-full",
        toneDotClasses[normalizedTone]
      )}
    />
  );
}

/**
 * Custom listbox-backed select menu.
 *
 * `onChange` receives the selected option value. Root passthrough props are
 * limited to `id`, `title`, `data-*`, and `aria-*`; event handlers are
 * intentionally not spread.
 */
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
  size = "md",
  placeholder = "",
  searchable = false,
  searchAriaLabel = "Search options",
  searchPlaceholder = "Search options",
  ...rest
}: SelectMenuProps) {
  const [open, setOpen] = React.useState(false);
  const [searchQuery, setSearchQuery] = React.useState("");
  const normalizedSearchQuery = searchQuery.trim().toLowerCase();
  const visibleOptions = normalizedSearchQuery
    ? options.filter((option) =>
        [optionLabel(option), option.value].some((candidate) =>
          String(candidate).toLowerCase().includes(normalizedSearchQuery)
        )
      )
    : options;
  const [activeIndex, setActiveIndex] = React.useState(() =>
    selectedOptionIndex(visibleOptions, value)
  );
  const rootRef = React.useRef<HTMLDivElement>(null);
  const buttonRef = React.useRef<HTMLButtonElement>(null);
  const idRef = React.useRef("");
  const restoreFocusOnCloseRef = React.useRef(false);
  const wasOpenRef = React.useRef(open);
  const outsideClickEntryRef = React.useRef<OpenSelectMenuEntry | null>(null);
  if (!idRef.current) idRef.current = createSelectMenuId();

  const selectedIndex = selectedOptionIndex(options, value);
  const selectedOption = selectedIndex >= 0 ? options[selectedIndex] : null;
  const selectedLabel = optionLabel(selectedOption, placeholder);
  const listboxId = `${idRef.current}-listbox`;
  const activeOptionId =
    open && activeIndex >= 0 && activeIndex < visibleOptions.length
      ? `${idRef.current}-option-${activeIndex}`
      : null;
  const effectiveAriaLabel = ariaLabel || ariaLabelProp;
  const effectiveAlign = normalizeAlign(align);
  const effectiveSize = sizeClasses[size] || sizeClasses.md;
  const hasEnabledOption = firstEnabledIndex(options) >= 0;
  const interactionDisabled = disabled || !hasEnabledOption;
  const optionsKey = optionsIdentity(visibleOptions);
  const rootPassthroughProps = safeRootProps(rest);
  const buttonListboxProps = {
    ...(open ? { "aria-controls": listboxId } : {}),
    ...(activeOptionId && !searchable
      ? { "aria-activedescendant": activeOptionId }
      : {}),
  };

  const closeMenu = ({ restoreFocus = true }: { restoreFocus?: boolean } = {}) => {
    restoreFocusOnCloseRef.current = restoreFocus;
    setOpen(false);
  };

  if (!outsideClickEntryRef.current) {
    outsideClickEntryRef.current = {
      rootRef,
      close: () => {
        restoreFocusOnCloseRef.current = false;
        setOpen(false);
      },
    };
  }

  React.useEffect(() => {
    setActiveIndex(selectedOptionIndex(visibleOptions, value));
  }, [optionsKey, value, normalizedSearchQuery]);

  React.useEffect(() => {
    if (!open && searchQuery) setSearchQuery("");
  }, [open, searchQuery]);

  React.useEffect(() => {
    if (!open) return undefined;
    const outsideClickEntry = outsideClickEntryRef.current;
    if (!outsideClickEntry) return undefined;
    return registerOpenSelectMenu(outsideClickEntry);
  }, [open]);

  React.useEffect(() => {
    if (wasOpenRef.current && !open && restoreFocusOnCloseRef.current) {
      buttonRef.current?.focus?.();
    }
    if (open) restoreFocusOnCloseRef.current = false;
    wasOpenRef.current = open;
  }, [open]);

  const chooseOption = (option: SelectMenuOption | undefined) => {
    if (!option || option.disabled) return;
    closeMenu();
    if (option.value !== value) onChange(option.value);
  };

  const openWithIndex = (index: number) => {
    if (interactionDisabled || index < 0) return;
    setActiveIndex(
      index < visibleOptions.length ? index : firstEnabledIndex(visibleOptions)
    );
    setOpen(true);
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (interactionDisabled) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const direction: 1 | -1 = event.key === "ArrowDown" ? 1 : -1;
      const baseIndex = open
        ? activeIndex
        : selectedOptionIndex(visibleOptions, value);
      const nextIndex = nextEnabledIndex(visibleOptions, baseIndex, direction);
      openWithIndex(nextIndex);
      return;
    }

    if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      const direction: 1 | -1 = event.key === "Home" ? 1 : -1;
      openWithIndex(edgeEnabledIndex(visibleOptions, direction));
      return;
    }

    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      if (!open) {
        openWithIndex(selectedIndex);
        return;
      }
      chooseOption(visibleOptions[activeIndex]);
      return;
    }

    if (event.key === "Escape") {
      if (open) {
        event.preventDefault();
        event.stopPropagation();
        closeMenu();
      }
      return;
    }

    if (event.key === "Tab") closeMenu({ restoreFocus: false });
  };

  const handleSearchKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (
      event.key === "ArrowDown" ||
      event.key === "ArrowUp" ||
      event.key === "Enter" ||
      event.key === "Escape"
    ) {
      handleKeyDown(event);
    }
  };

  const handleRootBlur = (event: FocusEvent<HTMLDivElement>) => {
    if (open && !event.currentTarget.contains(event.relatedTarget)) {
      closeMenu({ restoreFocus: false });
    }
  };

  return (
    <div
      ref={rootRef}
      className={cn(
        "relative inline-block text-left font-sans",
        effectiveSize.root,
        className
      )}
      {...rootPassthroughProps}
      onBlur={handleRootBlur}
    >
      <button
        ref={buttonRef}
        type="button"
        aria-haspopup="listbox"
        aria-expanded={open ? "true" : "false"}
        aria-label={effectiveAriaLabel}
        aria-labelledby={ariaLabelledBy}
        {...buttonListboxProps}
        disabled={interactionDisabled}
        onClick={() =>
          !interactionDisabled &&
          setOpen((current) => {
            restoreFocusOnCloseRef.current = false;
            if (!current) setActiveIndex(selectedOptionIndex(visibleOptions, value));
            return !current;
          })}
        onKeyDown={handleKeyDown}
        className={cn(
          "inline-flex w-full items-center justify-between gap-2 rounded-[8px] border",
          "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]",
          "text-[var(--v2-text-strong)] shadow-none transition-colors",
          "hover:bg-[var(--v2-surface-muted)]",
          "focus-visible:outline-none focus-visible:ring-2",
          "focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_32%,transparent)]",
          "disabled:cursor-not-allowed disabled:opacity-60",
          effectiveSize.button,
          buttonClassName
        )}
      >
        <span className="flex min-w-0 items-center gap-2">
          <ToneDot tone={selectedOption?.tone} />
          <span className="truncate">{selectedLabel}</span>
        </span>
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
            "absolute top-[calc(100%+0.35rem)] z-30 min-w-full overflow-hidden rounded-[10px]",
            "border border-[color-mix(in_srgb,var(--v2-text-strong)_16%,var(--v2-panel-border))]",
            "bg-[color-mix(in_srgb,var(--v2-canvas-strong)_92%,var(--v2-surface))] p-1",
            "shadow-[0_30px_72px_-18px_rgba(0,0,0,0.86),0_10px_24px_-18px_rgba(0,0,0,0.68)]",
            "ring-1 ring-[color-mix(in_srgb,var(--v2-text-strong)_8%,transparent)]",
            alignClasses[effectiveAlign],
            menuClassName
          )}
        >
          {searchable && (
            <input
              type="search"
              role="combobox"
              value={searchQuery}
              aria-label={searchAriaLabel}
              aria-autocomplete="list"
              aria-expanded="true"
              aria-controls={listboxId}
              aria-activedescendant={activeOptionId ?? undefined}
              placeholder={searchPlaceholder}
              autoFocus
              onChange={(event) => setSearchQuery(event.currentTarget.value)}
              onKeyDown={handleSearchKeyDown}
              className={cn(
                "sticky top-0 z-10 mb-1 h-9 w-full rounded-[7px] border px-2.5",
                "border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)]",
                "text-[var(--v2-text-strong)] placeholder:text-[var(--v2-text-faint)]",
                "focus-visible:outline-none focus-visible:ring-2",
                "focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_32%,transparent)]"
              )}
            />
          )}
          <div id={listboxId} role="listbox">
            {visibleOptions.map((option, index) => {
              const isSelected = option.value === value;
              const isActive = index === activeIndex;
              return (
                <button
                  key={option.value}
                  id={`${idRef.current}-option-${index}`}
                  type="button"
                  role="option"
                  aria-selected={isSelected ? "true" : "false"}
                  aria-disabled={option.disabled ? "true" : "false"}
                  disabled={option.disabled}
                  onMouseEnter={() => !option.disabled && setActiveIndex(index)}
                  onClick={() => chooseOption(option)}
                  className={cn(
                    "flex w-full items-center justify-between gap-3 rounded-[7px]",
                    "text-left text-[var(--v2-text)] transition-colors",
                    "focus-visible:outline-none",
                    "focus-visible:ring-2 focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_30%,transparent)]",
                    "disabled:cursor-not-allowed disabled:opacity-50",
                    isActive
                      ? "bg-[var(--v2-surface-muted)] text-[var(--v2-text-strong)]"
                      : isSelected
                        ? "bg-[var(--v2-accent-soft)] text-[var(--v2-text-strong)]"
                        : "hover:bg-[var(--v2-surface-soft)]",
                    effectiveSize.option,
                    optionClassName
                  )}
                >
                  <span className="flex min-w-0 items-center gap-2">
                    <ToneDot tone={option.tone} />
                    <span className="truncate">{optionLabel(option)}</span>
                  </span>
                  {isSelected && (
                    <Icon
                      name="check"
                      className="h-3.5 w-3.5 shrink-0 text-[var(--v2-accent-text)]"
                    />
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
