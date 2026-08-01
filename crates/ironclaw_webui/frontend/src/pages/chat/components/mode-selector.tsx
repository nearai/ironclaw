/**
 * ModeSelector — the composer autonomy pill. Controlled: the parent owns the
 * value (via `useAgentMode`) so the same source of truth can later be mirrored
 * into settings. Opens *upward* because the composer sits near the bottom edge.
 *
 * The four modes escalate how much the agent may do without asking; Bypass gets
 * a caution treatment because it removes every approval.
 */
import React from "react";
import { Icon } from "../../../design-system/icons";
import { useT } from "../../../lib/i18n";
import {
  AGENT_MODE_ORDER,
  type AgentMode,
} from "../lib/agent-mode";

interface ModeMeta {
  icon: string;
  labelKey: string;
  descKey: string;
  caution?: boolean;
}

const MODE_META: Record<AgentMode, ModeMeta> = {
  suggest: {
    icon: "shield",
    labelKey: "mode.suggest.label",
    descKey: "mode.suggest.desc",
  },
  plan: {
    icon: "list",
    labelKey: "mode.plan.label",
    descKey: "mode.plan.desc",
  },
  auto: {
    icon: "spark",
    labelKey: "mode.auto.label",
    descKey: "mode.auto.desc",
  },
  bypass: {
    icon: "bolt",
    labelKey: "mode.bypass.label",
    descKey: "mode.bypass.desc",
    caution: true,
  },
};

export function ModeSelector({
  mode,
  onChange,
  disabled = false,
}: {
  mode: AgentMode;
  onChange: (mode: AgentMode) => void;
  disabled?: boolean;
}) {
  const t = useT();
  const [open, setOpen] = React.useState(false);
  const rootRef = React.useRef<HTMLDivElement | null>(null);
  const current = MODE_META[mode] ?? MODE_META.suggest;

  React.useEffect(() => {
    if (!open) return undefined;
    const onDocMouseDown = (event: MouseEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDocMouseDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onDocMouseDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  const choose = (next: AgentMode) => {
    setOpen(false);
    if (next !== mode) onChange(next);
  };

  return (
    <div ref={rootRef} className="relative">
      <button
        type="button"
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="listbox"
        aria-expanded={open ? "true" : "false"}
        aria-label={t("mode.ariaLabel", { mode: t(current.labelKey) })}
        data-testid="mode-selector"
        className={[
          "inline-flex h-7 items-center gap-1 rounded-full border px-2 text-[11px] font-medium transition-colors",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[color-mix(in_srgb,var(--v2-accent)_32%,transparent)]",
          "disabled:cursor-not-allowed disabled:opacity-50",
          current.caution
            ? "border-[color-mix(in_srgb,var(--v2-warning-text)_40%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]"
            : "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)] hover:text-[var(--v2-text-strong)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",
        ].join(" ")}
      >
        <Icon name={current.icon} className="h-3 w-3" />
        <span>{t(current.labelKey)}</span>
        <Icon
          name="chevron"
          className={["h-2.5 w-2.5 opacity-70 transition-transform", open && "rotate-180"]
            .filter(Boolean)
            .join(" ")}
        />
      </button>

      {open && (
        <div
          role="listbox"
          aria-label={t("mode.menuLabel")}
          className="absolute bottom-[calc(100%+0.5rem)] left-0 z-40 w-[19rem] overflow-hidden rounded-[14px] border border-[color-mix(in_srgb,var(--v2-text-strong)_14%,var(--v2-panel-border))] bg-[color-mix(in_srgb,var(--v2-canvas-strong)_94%,var(--v2-surface))] p-1.5 shadow-[0_30px_72px_-18px_rgba(0,0,0,0.86)]"
        >
          <div className="px-2 pb-1.5 pt-1 font-mono text-[0.625rem] font-semibold uppercase tracking-[0.18em] text-[var(--v2-text-faint)]">
            {t("mode.menuLabel")}
          </div>
          {AGENT_MODE_ORDER.map((value) => {
            const meta = MODE_META[value];
            const selected = value === mode;
            return (
              <button
                key={value}
                type="button"
                role="option"
                aria-selected={selected ? "true" : "false"}
                onClick={() => choose(value)}
                className={[
                  "flex w-full items-start gap-2.5 rounded-[10px] px-2 py-2 text-left transition-colors",
                  selected
                    ? "bg-[var(--v2-accent-soft)]"
                    : "hover:bg-[var(--v2-surface-soft)]",
                ].join(" ")}
              >
                <span
                  className={[
                    "mt-0.5 grid h-6 w-6 shrink-0 place-items-center rounded-[8px] border",
                    meta.caution
                      ? "border-[color-mix(in_srgb,var(--v2-warning-text)_36%,var(--v2-panel-border))] text-[var(--v2-warning-text)]"
                      : "border-[var(--v2-panel-border)] text-[var(--v2-text-muted)]",
                  ].join(" ")}
                >
                  <Icon name={meta.icon} className="h-3.5 w-3.5" />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-2">
                    <span className="text-sm font-semibold text-[var(--v2-text-strong)]">
                      {t(meta.labelKey)}
                    </span>
                    {selected && (
                      <Icon
                        name="check"
                        className="h-3.5 w-3.5 text-[var(--v2-accent-text)]"
                      />
                    )}
                  </span>
                  <span className="mt-0.5 block text-xs leading-5 text-[var(--v2-text-muted)]">
                    {t(meta.descKey)}
                  </span>
                </span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
