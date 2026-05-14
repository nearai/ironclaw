import { React, html } from "../../lib/html.js";
import { useT } from "../../lib/i18n.js";
import { useLogs } from "./hooks/useLogs.js";

const LEVELS = ["all", "trace", "debug", "info", "warn", "error"];
const SERVER_LEVELS = ["trace", "debug", "info", "warn", "error"];

const LEVEL_COLORS = {
  trace: "text-[var(--v2-text-muted)]",
  debug: "text-[color-mix(in_srgb,var(--v2-accent)_80%,white)]",
  info: "text-[var(--v2-text-strong)]",
  warn: "text-yellow-400",
  error: "text-red-400",
};

const LEVEL_BG = {
  warn: "bg-yellow-500/5",
  error: "bg-red-500/8",
};

function LogEntry({ entry }) {
  const [expanded, setExpanded] = React.useState(false);
  const ts = entry.timestamp ? entry.timestamp.substring(11, 23) : "";
  const levelColor = LEVEL_COLORS[entry.level] || LEVEL_COLORS.info;
  const rowBg = LEVEL_BG[entry.level] || "";

  return html`
    <div
      onClick=${() => setExpanded((v) => !v)}
      className=${[
        "grid cursor-pointer select-none gap-x-3 px-4 py-1 font-mono text-xs hover:bg-[var(--v2-surface-muted)]",
        "grid-cols-[7rem_3rem_minmax(10rem,18rem)_1fr]",
        rowBg,
      ]
        .filter(Boolean)
        .join(" ")}
    >
      <span className="text-[var(--v2-text-muted)] tabular-nums">${ts}</span>
      <span className=${["font-semibold uppercase", levelColor].join(" ")}>
        ${entry.level}
      </span>
      <span className="truncate text-[var(--v2-text-muted)]">${entry.target}</span>
      <span
        className=${[
          "min-w-0 text-[var(--v2-text-base)]",
          expanded ? "whitespace-pre-wrap break-all" : "truncate",
        ].join(" ")}
      >
        ${entry.message}
      </span>
    </div>
  `;
}

function ToolbarSelect({ value, onChange, options, labelKey, t }) {
  return html`
    <select
      value=${value}
      onChange=${(e) => onChange(e.target.value)}
      className="v2-select h-8 min-w-0 rounded-[8px] px-2.5 py-0 text-xs"
    >
      ${options.map(
        (opt) => html`<option key=${opt} value=${opt}>${t(labelKey(opt))}</option>`
      )}
    </select>
  `;
}

export function LogsPage() {
  const t = useT();
  const {
    entries,
    totalCount,
    paused,
    togglePause,
    clearEntries,
    levelFilter,
    setLevelFilter,
    targetFilter,
    setTargetFilter,
    autoScroll,
    setAutoScroll,
    serverLevel,
    changeServerLevel,
  } = useLogs();

  const outputRef = React.useRef(null);

  React.useEffect(() => {
    if (autoScroll && outputRef.current) {
      outputRef.current.scrollTop = 0;
    }
  }, [entries, autoScroll]);

  return html`
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <!-- Toolbar -->
      <div
        className="flex shrink-0 flex-wrap items-center gap-2 border-b border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] px-4 py-2"
      >
        <!-- Level filter -->
        <${ToolbarSelect}
          value=${levelFilter}
          onChange=${setLevelFilter}
          options=${LEVELS}
          labelKey=${(opt) => (opt === "all" ? "logs.levelAll" : `logs.level.${opt}`)}
          t=${t}
        />

        <!-- Target filter -->
        <input
          type="text"
          value=${targetFilter}
          onInput=${(e) => setTargetFilter(e.target.value)}
          placeholder=${t("logs.filterTarget")}
          className="h-8 min-w-[10rem] flex-1 rounded-[8px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-3 text-xs text-[var(--v2-text-base)] placeholder:text-[var(--v2-text-muted)] focus:outline-none focus:ring-1 focus:ring-[var(--v2-accent)]"
        />

        <div className="flex items-center gap-2 ml-auto">
          <!-- Auto-scroll toggle -->
          <label className="flex cursor-pointer items-center gap-1.5 text-xs text-[var(--v2-text-muted)]">
            <input
              type="checkbox"
              checked=${autoScroll}
              onChange=${(e) => setAutoScroll(e.target.checked)}
              className="h-3.5 w-3.5 accent-[var(--v2-accent)]"
            />
            ${t("logs.autoScroll")}
          </label>

          <!-- Pause/Resume -->
          <button
            onClick=${togglePause}
            className=${[
              "h-8 rounded-[8px] px-3 text-xs font-medium",
              paused
                ? "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)] hover:bg-[color-mix(in_srgb,var(--v2-accent)_18%,transparent)]"
                : "border border-[var(--v2-panel-border)] text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",
            ].join(" ")}
          >
            ${paused ? t("logs.resume") : t("logs.pause")}
          </button>

          <!-- Clear -->
          <button
            onClick=${() => {
              if (confirm(t("logs.confirmClear"))) clearEntries();
            }}
            className="h-8 rounded-[8px] border border-[var(--v2-panel-border)] px-3 text-xs text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          >
            ${t("logs.clear")}
          </button>
        </div>

        <!-- Server log level -->
        ${serverLevel != null &&
        html`
          <div className="flex w-full items-center gap-2 border-t border-[var(--v2-panel-border)] pt-2 text-xs text-[var(--v2-text-muted)]">
            <span>${t("logs.serverLevel")}</span>
            <${ToolbarSelect}
              value=${serverLevel}
              onChange=${changeServerLevel}
              options=${SERVER_LEVELS}
              labelKey=${(opt) => `logs.level.${opt}`}
              t=${t}
            />
            <span className="ml-auto tabular-nums">
              ${t("logs.entryCount", { count: totalCount })}
              ${paused ? html`<span className="ml-1 text-yellow-400">${t("logs.pausedBadge")}</span>` : null}
            </span>
          </div>
        `}
      </div>

      <!-- Log output -->
      <div
        ref=${outputRef}
        className="min-h-0 flex-1 overflow-y-auto bg-[var(--v2-canvas)]"
      >
        ${entries.length === 0
          ? html`
              <div
                className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
              >
                ${t("logs.empty")}
              </div>
            `
          : entries.map(
              (entry, i) => html`<${LogEntry} key=${i} entry=${entry} />`
            )}
      </div>
    </div>
  `;
}
