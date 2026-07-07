import { React, html } from "../lib/html.js";
import { cn } from "../utils/cn.js";
import { Icon } from "./icons.js";

let nextSelectMenuId = 0;

const toneDotClasses = {
  neutral: "bg-[var(--v2-text-faint)]",
  positive: "bg-[var(--v2-positive-text)]",
  warning: "bg-[var(--v2-warning-text)]",
  danger: "bg-[var(--v2-danger-text)]",
  info: "bg-[var(--v2-info-text)]",
  accent: "bg-[var(--v2-accent-text)]",
};

function createSelectMenuId() {
  nextSelectMenuId += 1;
  return `v2-select-menu-${nextSelectMenuId}`;
}

function firstEnabledIndex(options) {
  return options.findIndex((option) => !option.disabled);
}

function selectedOptionIndex(options, value) {
  const index = options.findIndex((option) => option.value === value);
  return index >= 0 ? index : firstEnabledIndex(options);
}

function nextEnabledIndex(options, currentIndex, direction) {
  if (!options.length) return -1;
  const start =
    currentIndex >= 0 ? currentIndex : direction > 0 ? -1 : options.length;
  for (let step = 1; step <= options.length; step += 1) {
    const index = (start + direction * step + options.length) % options.length;
    if (!options[index]?.disabled) return index;
  }
  return currentIndex;
}

function edgeEnabledIndex(options, direction) {
  if (!options.length) return -1;
  const start = direction > 0 ? 0 : options.length - 1;
  const end = direction > 0 ? options.length : -1;
  for (let index = start; index !== end; index += direction) {
    if (!options[index]?.disabled) return index;
  }
  return -1;
}

function optionLabel(option, fallback = "") {
  return option?.label ?? option?.value ?? fallback;
}

function ToneDot({ tone }) {
  if (!tone) return null;
  return html`
    <span
      aria-hidden="true"
      className=${cn(
        "h-1.5 w-1.5 shrink-0 rounded-full",
        toneDotClasses[tone] ?? toneDotClasses.neutral
      )}
    />
  `;
}

export function SelectMenu({
  value,
  options = [],
  onChange = () => {},
  disabled = false,
  ariaLabel,
  "aria-label": ariaLabelProp,
  "aria-labelledby": ariaLabelledBy,
  className = "",
  buttonClassName = "",
  menuClassName = "",
  optionClassName = "",
  align = "right",
  placeholder = "",
  ...rest
}) {
  const [open, setOpen] = React.useState(false);
  const [activeIndex, setActiveIndex] = React.useState(() =>
    selectedOptionIndex(options, value)
  );
  const rootRef = React.useRef(null);
  const idRef = React.useRef("");
  if (!idRef.current) idRef.current = createSelectMenuId();

  const selectedIndex = selectedOptionIndex(options, value);
  const selectedOption = selectedIndex >= 0 ? options[selectedIndex] : null;
  const selectedLabel = optionLabel(selectedOption, placeholder);
  const listboxId = `${idRef.current}-listbox`;
  const effectiveAriaLabel = ariaLabel || ariaLabelProp;

  React.useEffect(() => {
    if (!open || typeof document === "undefined") return undefined;
    const handleDocumentMouseDown = (event) => {
      if (rootRef.current?.contains?.(event.target)) return;
      setOpen(false);
    };
    document.addEventListener("mousedown", handleDocumentMouseDown);
    return () => document.removeEventListener("mousedown", handleDocumentMouseDown);
  }, [open]);

  const chooseOption = (option) => {
    if (!option || option.disabled) return;
    setOpen(false);
    if (option.value !== value) onChange(option.value, option);
  };

  const openWithIndex = (index) => {
    if (disabled) return;
    setActiveIndex(index);
    setOpen(true);
  };

  const handleKeyDown = (event) => {
    if (disabled) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const direction = event.key === "ArrowDown" ? 1 : -1;
      const nextIndex = nextEnabledIndex(options, activeIndex, direction);
      openWithIndex(nextIndex);
      return;
    }

    if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      const direction = event.key === "Home" ? 1 : -1;
      openWithIndex(edgeEnabledIndex(options, direction));
      return;
    }

    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      if (!open) {
        openWithIndex(selectedIndex);
        return;
      }
      chooseOption(options[activeIndex]);
      return;
    }

    if (event.key === "Escape") {
      event.preventDefault();
      setOpen(false);
      return;
    }

    if (event.key === "Tab") setOpen(false);
  };

  return html`
    <div
      ref=${rootRef}
      className=${cn("relative inline-block min-w-[9.5rem] text-left", className)}
      ...${rest}
    >
      <button
        type="button"
        aria-haspopup="listbox"
        aria-expanded=${open ? "true" : "false"}
        aria-controls=${open ? listboxId : undefined}
        aria-label=${effectiveAriaLabel}
        aria-labelledby=${ariaLabelledBy}
        disabled=${disabled}
        onClick=${() =>
          !disabled &&
          setOpen((current) => {
            if (!current) setActiveIndex(selectedIndex);
            return !current;
          })}
        onKeyDown=${handleKeyDown}
        className=${cn(
          "inline-flex h-8 w-full items-center justify-between gap-2 rounded-[8px] border",
          "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-2.5",
          "font-mono text-xs text-[var(--v2-text-strong)] shadow-none transition-colors",
          "hover:bg-[var(--v2-surface-muted)]",
          "focus-visible:outline-none focus-visible:ring-2",
          "focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_32%,transparent)]",
          "disabled:cursor-not-allowed disabled:opacity-60",
          buttonClassName
        )}
      >
        <span className="flex min-w-0 items-center gap-2">
          <${ToneDot} tone=${selectedOption?.tone} />
          <span className="truncate">${selectedLabel}</span>
        </span>
        <${Icon}
          name="chevron"
          className=${cn(
            "h-3.5 w-3.5 shrink-0 text-[var(--v2-text-faint)] transition-transform",
            open && "rotate-180"
          )}
        />
      </button>

      ${open &&
      html`
        <div
          id=${listboxId}
          role="listbox"
          aria-label=${effectiveAriaLabel}
          aria-labelledby=${ariaLabelledBy}
          className=${cn(
            "absolute top-[calc(100%+0.35rem)] z-30 min-w-full overflow-hidden rounded-[10px]",
            "border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] p-1",
            "shadow-[0_22px_48px_-22px_rgba(0,0,0,0.78)]",
            align === "left" ? "left-0" : "right-0",
            menuClassName
          )}
        >
          ${options.map((option, index) => {
            const isSelected = option.value === value;
            const isActive = index === activeIndex;
            return html`
              <button
                key=${option.value}
                id=${`${idRef.current}-option-${index}`}
                type="button"
                role="option"
                aria-selected=${isSelected ? "true" : "false"}
                aria-disabled=${option.disabled ? "true" : "false"}
                disabled=${option.disabled}
                onMouseEnter=${() => !option.disabled && setActiveIndex(index)}
                onClick=${() => chooseOption(option)}
                className=${cn(
                  "flex w-full items-center justify-between gap-3 rounded-[7px] px-2.5 py-2",
                  "text-left font-mono text-xs text-[var(--v2-text)] transition-colors",
                  "hover:bg-[var(--v2-surface-soft)] focus-visible:outline-none",
                  "focus-visible:ring-2 focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_30%,transparent)]",
                  "disabled:cursor-not-allowed disabled:opacity-50",
                  isSelected &&
                    "bg-[var(--v2-accent-soft)] text-[var(--v2-text-strong)]",
                  isActive &&
                    "bg-[var(--v2-surface-muted)] text-[var(--v2-text-strong)]",
                  optionClassName
                )}
              >
                <span className="flex min-w-0 items-center gap-2">
                  <${ToneDot} tone=${option.tone} />
                  <span className="truncate">${optionLabel(option)}</span>
                </span>
                ${isSelected &&
                html`<${Icon}
                  name="check"
                  className="h-3.5 w-3.5 shrink-0 text-[var(--v2-accent-text)]"
                />`}
              </button>
            `;
          })}
        </div>
      `}
    </div>
  `;
}
