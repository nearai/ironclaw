// @ts-nocheck
import { useOutletContext } from "react-router";
import React from "react";
import { Button, Callout, SearchInput, Select, Spinner } from "@ironclaw/ui";
import { useT } from "../../lib/i18n";
import { useLogs } from "./hooks/useLogs";

const LEVELS = ["all", "trace", "debug", "info", "warn", "error"];
const SERVER_LEVELS = ["trace", "debug", "info", "warn", "error"];

const LEVEL_COLORS = {
  trace: "text-[var(--v2-text-muted)]",
  debug: "text-[color-mix(in_srgb,var(--v2-accent)_80%,white)]",
  info: "text-[var(--v2-text-strong)]",
  warn: "text-[var(--v2-warning-text)]",
  error: "text-[var(--v2-danger-text)]",
};

const LEVEL_BG = {
  warn: "bg-[color-mix(in_srgb,var(--v2-warning-text)_5%,transparent)]",
  error: "bg-[color-mix(in_srgb,var(--v2-danger-text)_7%,transparent)]",
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

  return (
    <div data-testid="logs-entry" className={rowBg}>
      <div
        data-testid="logs-entry-row"
        onClick={(event) => {
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
        className={[
          "grid cursor-pointer select-text gap-x-3 px-4 py-1 font-mono text-xs hover:bg-[var(--v2-surface-muted)]",
          "grid-cols-[7rem_3rem_minmax(10rem,18rem)_1fr]",
        ].join(" ")}
      >
        <span className="text-[var(--v2-text-muted)] tabular-nums">{ts}</span>
        <span className={["font-semibold uppercase", levelColor].join(" ")}>
          {entry.level}
        </span>
        <span className="truncate text-[var(--v2-text-muted)]">{entry.target}</span>
        <span
          data-testid="logs-entry-message"
          className={[
            "min-w-0 text-[var(--v2-text-base)]",
            expanded ? "whitespace-pre-wrap break-all" : "truncate",
          ].join(" ")}
        >
          {entry.message}
        </span>
      </div>
      {expanded && contextItems.length > 0 &&
      (
        <div
          data-testid="logs-entry-context"
          className="flex flex-wrap gap-1.5 px-4 pb-2 pl-[calc(7rem+3rem+2.5rem)] font-mono text-[11px] text-[var(--v2-text-muted)]"
        >
          {contextItems.map(
            (item) => (
              <span
                key={item.key}
                data-testid="logs-context-chip"
                data-context-key={item.key}
                className="inline-flex max-w-full items-center gap-1 rounded-[6px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-2 py-0.5"
              >
                <span>{t(item.labelKey)}</span>
                <span className="max-w-[18rem] truncate text-[var(--v2-text-base)]">{item.value}</span>
              </span>
            )
          )}
        </div>
      )}
    </div>
  );
}

function ToolbarSelect({ value, onChange, options, labelKey, label, t }) {
  // DS Select stretches to its container; the toolbar wants a fixed column.
  return (
    <div className="w-36 shrink-0">
      <Select
        size="sm"
        value={value}
        onChange={(e) => onChange(e.currentTarget.value)}
        aria-label={label}
        className="text-xs"
      >
        {options.map(
          (opt) => (<option key={opt} value={opt}>{t(labelKey(opt))}</option>)
        )}
      </Select>
    </div>
  );
}

function ScopeChip({ label, value, scopeKey }) {
  return (
    <span
      data-testid="logs-scope-chip"
      data-scope-key={scopeKey}
      className="inline-flex max-w-full items-center gap-1 rounded-[6px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)] px-2 py-1 font-mono text-[11px] text-[var(--v2-text-muted)]"
      title={`${label}: ${value}`}
    >
      <span className="uppercase tracking-[0.08em]">{label}</span>
      <span className="max-w-[18rem] truncate text-[var(--v2-text-base)]">{value}</span>
    </span>
  );
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

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      {/* Toolbar */}
      <div
        className="flex shrink-0 flex-wrap items-center gap-2 border-b border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] px-4 py-2"
      >
        {/* Level filter */}
        <ToolbarSelect
          value={levelFilter}
          onChange={setLevelFilter}
          options={LEVELS}
          labelKey={(opt) => (opt === "all" ? "logs.levelAll" : `logs.level.${opt}`)}
          label={t("logs.levelAll")}
          t={t}
        />

        {/* Target filter */}
        <SearchInput
          label={t("logs.filterTarget")}
          value={targetFilter}
          onChange={(e) => setTargetFilter(e.currentTarget.value)}
          placeholder={t("logs.filterTarget")}
          className="min-w-[10rem] flex-1"
        />

        <div className="flex items-center gap-2 ml-auto">
          <span className="hidden tabular-nums text-xs text-[var(--v2-text-muted)] sm:inline">
            {t("logs.entryCount", { count: totalCount })}
          </span>

          {/* Auto-scroll toggle */}
          <label className="flex cursor-pointer items-center gap-1.5 text-xs text-[var(--v2-text-muted)]">
            <input
              type="checkbox"
              checked={autoScroll}
              onChange={(e) => setAutoScroll(e.target.checked)}
              className="h-3.5 w-3.5 accent-[var(--v2-accent)]"
            />
            {t("logs.autoScroll")}
          </label>

          {/* Pause/Resume */}
          <Button
            variant={paused ? "outline" : "secondary"}
            size="sm"
            aria-pressed={paused}
            onClick={togglePause}
          >
            {paused ? t("logs.resume") : t("logs.pause")}
          </Button>

          {/* Clear */}
          <Button
            variant="secondary"
            size="sm"
            onClick={() => {
              if (confirm(t("logs.confirmClear"))) clearEntries();
            }}
          >
            {t("logs.clear")}
          </Button>
        </div>

        {activeScope.length > 0 &&
        (
          <div
            data-testid="logs-scope-toolbar"
            className="flex w-full flex-wrap items-center gap-2 border-t border-[var(--v2-panel-border)] pt-2 text-xs text-[var(--v2-text-muted)]"
          >
            <span className="font-medium text-[var(--v2-text-strong)]">{t("logs.scoped")}</span>
            {activeScope.map(
              (item) => (<ScopeChip key={item.param} scopeKey={item.param} label={t(item.labelKey)} value={item.value} />)
            )}
            <a
              href="/logs"
              className="ml-auto rounded-[6px] px-2 py-1 text-xs text-[var(--v2-text-muted)] transition-colors hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-focus-ring)]"
            >
              {t("logs.clearScope")}
            </a>
          </div>
        )}

        {/* Server log level */}
        {serverLevel != null &&
        (
          <div className="flex w-full items-center gap-2 border-t border-[var(--v2-panel-border)] pt-2 text-xs text-[var(--v2-text-muted)]">
            <span>{t("logs.serverLevel")}</span>
            <ToolbarSelect
              value={serverLevel}
              onChange={changeServerLevel}
              options={SERVER_LEVELS}
              labelKey={(opt) => `logs.level.${opt}`}
              t={t}
            />
            <span className="ml-auto tabular-nums">
              {t("logs.entryCount", { count: totalCount })}
              {paused ? (<span className="ml-1 text-[var(--v2-warning-text)]">{t("logs.pausedBadge")}</span>) : null}
            </span>
          </div>
        )}
      </div>

      {/* Log output */}
      <div
        ref={outputRef}
        onScroll={handleOutputScroll}
        className="min-h-0 flex-1 overflow-y-auto bg-[var(--v2-canvas)]"
      >
        {error && hasEntries
          ? (
              <Callout tone="danger" className="sticky top-2 z-10 mx-3 mt-3 backdrop-blur">
                {t("error.loadFailed", {
                  what: t("nav.logs"),
                  message: error.message || error.statusText || "Request failed",
                })}
              </Callout>
            )
          : null}
        {needsThreadScope
          ? (
              <div
                data-testid="logs-select-thread-state"
                className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
              >
                {t("chat.selectConversation")}
              </div>
            )
          : error && !hasEntries
          ? (
              <div
                className="flex h-full items-center justify-center px-6 text-center text-sm text-[var(--v2-danger-text)]"
              >
                {t("error.loadFailed", {
                  what: t("nav.logs"),
                  message: error.message || error.statusText || "Request failed",
                })}
              </div>
            )
          : isLoading && !hasEntries
            ? (
                <div
                  className="flex h-full items-center justify-center gap-2 text-sm text-[var(--v2-text-muted)]"
                >
                  <Spinner label={t("common.loading")} />
                  {t("common.loading")}
                </div>
              )
            : !hasEntries
          ? (
              <div
                className="flex h-full items-center justify-center text-sm text-[var(--v2-text-muted)]"
              >
                {t("logs.empty")}
              </div>
            )
          : entries.map(
              (entry) => (<LogEntry key={entry.id} entry={entry} />)
            )}
      </div>
    </div>
  );
}
