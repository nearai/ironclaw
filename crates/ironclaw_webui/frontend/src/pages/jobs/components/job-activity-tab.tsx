import React from "react";
import { useT } from "../../../lib/i18n";
import { Button } from "@ironclaw/design-system";
import { EmptyPanel, Panel } from "@ironclaw/design-system";
import { formatJobDate } from "../lib/jobs-presenters";

const FILTERS = [
  { value: "all", label: "All events" },
  { value: "message", label: "Messages" },
  { value: "tool_use", label: "Tool calls" },
  { value: "tool_result", label: "Tool results" },
  { value: "status", label: "Status" },
  { value: "result", label: "Final results" },
];

function prettyJson(value) {
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function EventCard({ event }) {
  const { event_type: type, data } = event;

  if (type === "tool_use" || type === "tool_result") {
    return (
      <details className="rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3">
        <summary className="cursor-pointer list-none text-sm font-medium text-[var(--v2-text-strong)]">
          {type === "tool_use" ? data.tool_name || "Tool call" : data.tool_name || "Tool result"}
        </summary>
        <pre className="mt-3 overflow-x-auto whitespace-pre-wrap rounded-md bg-[var(--v2-code-bg)] p-3 font-mono text-xs leading-6 text-[var(--v2-text)]">{prettyJson(type === "tool_use" ? data.input : data.output || data.error || data)}</pre>
      </details>
    );
  }

  if (type === "message") {
    return (
      <div className="rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3">
        <div className="font-mono text-[11px] uppercase tracking-[var(--v2-tracking-caps)] text-[var(--v2-text-muted)]">{data.role || "assistant"}</div>
        <div className="mt-2 text-sm leading-6 text-[var(--v2-text-strong)]">{data.content || ""}</div>
      </div>
    );
  }

  return (
    <div className="rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3">
      <div className="font-mono text-[11px] uppercase tracking-[var(--v2-tracking-caps)] text-[var(--v2-text-muted)]">{type.replace(/_/g, " ")}</div>
      <div className="mt-2 text-sm leading-6 text-[var(--v2-text-strong)]">{data.message || data.status || prettyJson(data)}</div>
    </div>
  );
}

export function JobActivityTab({ job, events, onSendPrompt, isSendingPrompt }) {
  const t = useT();
  const [filter, setFilter] = React.useState("all");
  const [content, setContent] = React.useState("");
  const [autoScroll, setAutoScroll] = React.useState(true);
  const terminalRef = React.useRef(null);

  const filteredEvents = React.useMemo(
    () => (filter === "all" ? events : events.filter((event) => event.event_type === filter)),
    [events, filter]
  );

  React.useEffect(() => {
    if (autoScroll && terminalRef.current) {
      terminalRef.current.scrollTop = terminalRef.current.scrollHeight;
    }
  }, [autoScroll, filteredEvents.length]);

  const handleSend = React.useCallback(
    async (done = false) => {
      const trimmed = content.trim();
      if (!trimmed && !done) return;
      try {
        await onSendPrompt({ content: trimmed || "(done)", done });
        setContent("");
      } catch {
        // Mutation state drives the visible error banner.
      }
    },
    [content, onSendPrompt]
  );

  return (
    <Panel className="p-5 sm:p-6">
      <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
        <div>
          <div className="font-mono text-[11px] uppercase tracking-[var(--v2-tracking-caps)] text-[var(--v2-text-muted)]">Event stream</div>
          <h3 className="mt-2 text-xl font-medium text-[var(--v2-text-strong)]">Job activity</h3>
          <p className="mt-2 text-sm leading-6 text-[var(--v2-text-muted)]">Persisted events are refreshed automatically so operators can follow tool calls, prompts, and worker output.</p>
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <select
            value={filter}
            onChange={(event) => setFilter(event.currentTarget.value)}
            className="v2-select h-10 rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] px-3 text-sm text-[var(--v2-text-strong)] outline-none focus:border-[var(--v2-accent)]"
          >
            {FILTERS.map((option) => (<option key={option.value} value={option.value}>{option.label}</option>))}
          </select>
          <label className="flex items-center gap-2 text-sm text-[var(--v2-text-muted)]">
            <input type="checkbox" checked={autoScroll} onChange={(event) => setAutoScroll(event.target.checked)} />
            Auto-scroll
          </label>
        </div>
      </div>

      <div ref={terminalRef} className="mt-5 max-h-[56vh] space-y-3 overflow-y-auto rounded-[18px] border border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)]/78 p-4">
        {filteredEvents.length
          ? filteredEvents.map((event) => (
              <div key={event.id || `${event.event_type}-${event.created_at}`}>
                <div className="mb-2 font-mono text-[11px] uppercase tracking-[var(--v2-tracking-caps)] text-[var(--v2-text-muted)]">{formatJobDate(event.created_at)}</div>
                <EventCard event={event} />
              </div>
            ))
          : (
              <EmptyPanel
                title={t("job.noActivityTitle")}
                description={t("job.noActivityDesc")}
              />
            )}
      </div>

      {job.can_prompt && (
        <div className="mt-5 grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto_auto]">
          <input
            value={content}
            onInput={(event) => setContent(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.shiftKey) {
                event.preventDefault();
                handleSend(false);
              }
            }}
            placeholder={t("job.followupPlaceholder")}
            className="h-11 rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] px-3 text-sm text-[var(--v2-text-strong)] outline-none focus:border-[var(--v2-accent)]"
          />
          <Button variant="secondary" disabled={isSendingPrompt} onClick={() => handleSend(true)}>{t("common.done")}</Button>
          <Button variant="primary" disabled={isSendingPrompt} onClick={() => handleSend(false)}>{t("common.send")}</Button>
        </div>
      )}
    </Panel>
  );
}
