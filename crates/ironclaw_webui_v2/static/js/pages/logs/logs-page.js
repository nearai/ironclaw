import { useOutletContext } from "react-router";
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
  const t = useT();
  const [expanded, setExpanded] = React.useState(false);
  const ts = entry.timestamp ? entry.timestamp.substring(11, 23) : "";
  const levelColor = LEVEL_COLORS[entry.level] || LEVEL_COLORS.info;
  const rowBg = LEVEL_BG[entry.level] || "";
  const contextItems = [
    { key: "thread_id", labelKey: "logs.scope.thread", value: entry.threadId },
    { key: "run_id", labelKey: "logs.scope.run", value: entry.runId },
    { key: "turn_id", labelKey: "logs.scope.turn", value: entry.turnId },
    { key: "tool_call_id", labelKey: "logs.scope.toolCall", value: entry.toolCallId },
    { key: "tool_name", labelKey: "logs.scope.tool", value: entry.toolName },
    { key: "source", labelKey: "logs.scope.source", value: entry.source },
  ].filter((item) => Boolean(item.value));

  return html`
    <div data-testid="logs-entry" className=${rowBg}>
      <div
        data-testid="logs-entry-row"
        onClick=${(event) => {
          // Don't toggle when the click ends a text selection *within this row*
          // — otherwise selecting log text to copy it would also expand/collapse
          // the row. The selection is document-global, so scope the check to
          // event.currentTarget; a selection elsewhere on the page must not
          // block this row's toggle.
          const selection = typeof window !== "undefined" && window.getSelection?.();
          if (
            selection &&
            !selection.isCollapsed &&
            event.currentTarget.contains(selection.anchorNode) &&
            event.currentTarget.contains(selection.focusNode)
          ) {
            return;
          }
          setExpanded((v) => !v);
        }}
        className=${[
          "grid cursor-pointer select-text gap-x-3 px-4 py-1 font-mono text-xs hover:bg-[var(--v2-surface-muted)]",
          "grid-cols-[7rem_3rem_minmax(10rem,18rem)_1fr]",
        ].join(" ")}
      >
        <span className="text-[var(--v2-text-muted)] tabular-nums">${ts}</span>
        <span className=${["font-semibold uppercase", levelColor].join(" ")}>
          ${entry.level}
        </span>
        <span className="truncate text-[var(--v2-text-muted)]">${entry.target}</span>
        <span
          data-testid="logs-entry-message"
          className=${[
            "min-w-0 text-[var(--v2-text-base)]",
            expanded ? "whitespace-pre-wrap break-all" : "truncate",
          ].join(" ")}
        >
          ${entry.message}
        </span>
      </div>
      ${expanded && contextItems.length > 0 &&
      html`
        <div
          data-testid="logs-entry-context"
          className="flex flex-wrap gap-1.5 px-4 pb-2 pl-[calc(7rem+3rem+2.5rem)] font-mono text-[11px] text-[var(--v2-text-muted)]"
        >
          ${contextItems.map(
            (item) => html`
              <span
                key=${item.key}
                data-testid="logs-context-chip"
                data-context-key=${item.key}
                className="inline-flex max-w-full items-center gap-1 rounded-[6px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-2 py-0.5"
              >
                <span>${t(item.labelKey)}</span>
                <span className="max-w-[18rem] truncate text-[var(--v2-text-base)]">${item.value}</span>
              </span>
            `
          )}
        </div>
      `}
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

function ScopeChip({ label, value, scopeKey }) {
  return html`
    <span
      data-testid="logs-scope-chip"
      data-scope-key=${scopeKey}
      className="inline-flex max-w-full items-center gap-1 rounded-[6px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-2 py-1 font-mono text-[11px] text-[var(--v2-text-muted)]"
      title=${`${label}: ${value}`}
    >
      <span className="uppercase tracking-[0.08em]">${label}</span>
      <span className="max-w-[18rem] truncate text-[var(--v2-text-base)]">${value}</span>
    </span>
  `;
}

export function LogsPage() {
  const t = useT();
  const { isAdmin = false, threadsState } = useOutletContext() || {};
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
    scope,
    isLoading,
    error,
    needsThreadScope,
  } = useLogs({
    isAdmin,
    defaultThreadId: isAdmin ? null : threadsState?.activeThreadId || null,
  });

  const outputRef = React.useRef(null);
  const followLatestRef = React.useRef(true);

  React.useEffect(() => {
    if (autoScroll && followLatestRef.current && outputRef.current) {
      outputRef.current.scrollTop = 0;
    }
  }, [entries, autoScroll]);

  const handleOutputScroll = React.useCallback((event) => {
    followLatestRef.current = event.currentTarget.scrollTop <= 48;
  }, []);

  const hasEntries = entries.length > 0;
  const activeScope = scope?.active || [];

  return html`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
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
          <span className="hidden tabular-nums text-xs text-[var(--v2-text-muted)] sm:inline">
            ${t("logs.entryCount", { count: totalCount })}
          </span>

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

        ${activeScope.length > 0 &&
        html`
          <div
            data-testid="logs-scope-toolbar"
            className="flex w-full flex-wrap items-center gap-2 border-t border-[var(--v2-panel-border)] pt-2 text-xs text-[var(--v2-text-muted)]"
          >
            <span className="font-medium text-[var(--v2-text-strong)]">${t("logs.scoped")}</span>
            ${activeScope.map(
              (item) => html`<${ScopeChip} key=${item.param} scopeKey=${item.param} label=${t(item.labelKey)} value=${item.value} />`
            )}
            <a
              href="/v2/logs"
              className="ml-auto rounded-[6px] px-2 py-1 text-xs text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
            >
              ${t("logs.clearScope")}
            </a>
          </div>
        `}

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
        onScroll=${handleOutputScroll}
        className="min-h-0 flex-1 overflow-y-auto bg-[var(--v2-canvas)]"
      >
        ${error && hasEntries
          ? html`
              <div
                className="sticky top-0 z-10 border-b border-red-500/25 bg-red-950/70 px-4 py-2 text-xs text-red-100 backdrop-blur"
              >
                ${t("error.loadFailed", {
                  what: t("nav.logs"),
                  message: error.message || error.statusText || "Request failed",
                })}
              </div>
            `
          : null}
        ${needsThreadScope
          ? html`
              <div
                data-testid="logs-select-thread-state"
                className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
              >
                ${t("chat.selectConversation")}
              </div>
            `
          : error && !hasEntries
          ? html`
              <div
                className="flex h-full items-center justify-center px-6 text-center text-sm text-red-300"
              >
                ${t("error.loadFailed", {
                  what: t("nav.logs"),
                  message: error.message || error.statusText || "Request failed",
                })}
              </div>
            `
          : isLoading && !hasEntries
            ? html`
                <div
                  className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
                >
                  ${t("common.loading")}
                </div>
              `
            : !hasEntries
          ? html`
              <div
                className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
              >
                ${t("logs.empty")}
              </div>
            `
          : entries.map(
              (entry) => html`<${LogEntry} key=${entry.id} entry=${entry} />`
            )}
      </div>
    </div>
  `;
}
