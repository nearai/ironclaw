import { React, html } from "../../../lib/html.js";
import { Button } from "../../../design-system/button.js";
import { EmptyPanel, Panel } from "../../../design-system/primitives.js";
import { formatJobDate } from "../lib/jobs-presenters.js";

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
    return html`
      <details className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <summary className="cursor-pointer list-none text-sm font-semibold text-white">
          ${type === "tool_use" ? data.tool_name || "Tool call" : data.tool_name || "Tool result"}
        </summary>
        <pre className="mt-3 overflow-x-auto whitespace-pre-wrap rounded-md bg-iron-950/90 p-3 font-mono text-xs leading-6 text-iron-200">${prettyJson(type === "tool_use" ? data.input : data.output || data.error || data)}</pre>
      </details>
    `;
  }

  if (type === "message") {
    return html`
      <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
        <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${data.role || "assistant"}</div>
        <div className="mt-2 text-sm leading-6 text-iron-100">${data.content || ""}</div>
      </div>
    `;
  }

  return html`
    <div className="rounded-xl border border-white/10 bg-white/[0.03] px-4 py-3">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${type.replace(/_/g, " ")}</div>
      <div className="mt-2 text-sm leading-6 text-iron-100">${data.message || data.status || prettyJson(data)}</div>
    </div>
  `;
}

export function JobActivityTab({ job, events, onSendPrompt, isSendingPrompt }) {
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

  return html`
    <${Panel} className="p-5 sm:p-6">
      <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
        <div>
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Event stream</div>
          <h3 className="mt-2 text-xl font-semibold text-white">Job activity</h3>
          <p className="mt-2 text-sm leading-6 text-iron-300">Persisted events are refreshed automatically so operators can follow tool calls, prompts, and worker output.</p>
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <select
            value=${filter}
            onChange=${(event) => setFilter(event.target.value)}
            className="v2-select h-10 rounded-md border border-white/10 bg-iron-950/90 px-3 text-sm text-white outline-none focus:border-signal/45"
          >
            ${FILTERS.map((option) => html`<option key=${option.value} value=${option.value}>${option.label}</option>`)}
          </select>
          <label className="flex items-center gap-2 text-sm text-iron-300">
            <input type="checkbox" checked=${autoScroll} onChange=${(event) => setAutoScroll(event.target.checked)} />
            Auto-scroll
          </label>
        </div>
      </div>

      <div ref=${terminalRef} className="mt-5 max-h-[56vh] space-y-3 overflow-y-auto rounded-[18px] border border-white/10 bg-iron-950/78 p-4">
        ${filteredEvents.length
          ? filteredEvents.map((event) => html`
              <div key=${event.id || `${event.event_type}-${event.created_at}`}>
                <div className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${formatJobDate(event.created_at)}</div>
                <${EventCard} event=${event} />
              </div>
            `)
          : html`
              <${EmptyPanel}
                title="No activity captured yet"
                description="This job has not written any persisted events for the selected filter."
              />
            `}
      </div>

      ${job.can_prompt && html`
        <div className="mt-5 grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto_auto]">
          <input
            value=${content}
            onInput=${(event) => setContent(event.target.value)}
            onKeyDown=${(event) => {
              if (event.key === "Enter" && !event.shiftKey) {
                event.preventDefault();
                handleSend(false);
              }
            }}
            placeholder="Send a follow-up prompt to the running job"
            className="h-11 rounded-md border border-white/10 bg-iron-950/90 px-3 text-sm text-white outline-none focus:border-signal/45"
          />
          <${Button} variant="secondary" disabled=${isSendingPrompt} onClick=${() => handleSend(true)}>Done<//>
          <${Button} variant="primary" disabled=${isSendingPrompt} onClick=${() => handleSend(false)}>Send<//>
        </div>
      `}
    <//>
  `;
}
