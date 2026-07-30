import React from "react";
import { useT } from "../../../lib/i18n";
import {
  Button,
  CodePanel,
  EmptyPanel,
  Input,
  Card,
  SectionHeader,
  Select,
} from "@ironclaw/ui";
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
      <details className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <summary className="cursor-pointer list-none text-sm font-semibold text-white">
          {type === "tool_use" ? data.tool_name || "Tool call" : data.tool_name || "Tool result"}
        </summary>
        <CodePanel wrap className="mt-3">{prettyJson(type === "tool_use" ? data.input : data.output || data.error || data)}</CodePanel>
      </details>
    );
  }

  if (type === "message") {
    return (
      <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">{data.role || "assistant"}</div>
        <div className="mt-2 text-sm leading-6 text-iron-100">{data.content || ""}</div>
      </div>
    );
  }

  return (
    <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">{type.replace(/_/g, " ")}</div>
      <div className="mt-2 text-sm leading-6 text-iron-100">{data.message || data.status || prettyJson(data)}</div>
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
    <Card className="p-5 sm:p-6">
      <SectionHeader
        eyebrow="Event stream"
        title="Job activity"
        titleAs="h3"
        description="Persisted events are refreshed automatically so operators can follow tool calls, prompts, and worker output."
        actions={
          <>
            <div className="w-40">
              <Select
                size="sm"
                value={filter}
                onChange={(event) => setFilter(event.currentTarget.value)}
                aria-label={t("job.eventFilterLabel")}
              >
                {FILTERS.map((option) => (<option key={option.value} value={option.value}>{option.label}</option>))}
              </Select>
            </div>
            <label className="flex items-center gap-2 text-sm text-[var(--v2-text-muted)]">
              <input type="checkbox" checked={autoScroll} onChange={(event) => setAutoScroll(event.target.checked)} />
              Auto-scroll
            </label>
          </>
        }
      />

      <div ref={terminalRef} className="mt-5 max-h-[56vh] space-y-3 overflow-y-auto rounded-[18px] border border-white/10 bg-iron-950/78 p-4">
        {filteredEvents.length
          ? filteredEvents.map((event) => (
              <div key={event.id || `${event.event_type}-${event.created_at}`}>
                <div className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">{formatJobDate(event.created_at)}</div>
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
          <Input
            value={content}
            onChange={(event) => setContent(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.shiftKey) {
                event.preventDefault();
                handleSend(false);
              }
            }}
            placeholder={t("job.followupPlaceholder")}
          />
          <Button variant="secondary" disabled={isSendingPrompt} onClick={() => handleSend(true)}>{t("common.done")}</Button>
          <Button variant="primary" disabled={isSendingPrompt} onClick={() => handleSend(false)}>{t("common.send")}</Button>
        </div>
      )}
    </Card>
  );
}
