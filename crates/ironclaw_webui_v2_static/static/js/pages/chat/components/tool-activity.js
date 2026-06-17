import { Icon } from "../../../design-system/icons.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";

/* Status dot colour by tool status. Running shows the breathing dot (a no-op
   under the static motion policy, matching the Badge component's approach). */
const DOT_STYLE = {
  running: "bg-[var(--v2-accent)] animate-[v2-breathe_1.6s_ease-in-out_infinite]",
  success: "bg-[var(--v2-positive-text)]",
  error: "bg-[var(--v2-danger-text)]",
};

const STATUS_WORD = { success: "ok", error: "err", running: "run" };

/* Runs longer than this collapse into a single summary line. Runs of this
   length or shorter render each call as its own row. */
export const TOOL_RUN_COLLAPSE_AFTER = 2;

export function ToolActivity({ activity }) {
  if (activity.toolCalls && activity.toolCalls.length > 0) {
    return html`<${ToolRun} tools=${activity.toolCalls} />`;
  }
  return html`<${ToolActivityCard} activity=${activity} />`;
}

/* Categorise tool calls into files / searches / commands / other and build a
   compact human summary like "Explored 9 files, 5 searches, ran 2 commands".
   Best-effort keyword heuristic on the tool name. */
function summarizeTools(t, tools) {
  let files = 0;
  let searches = 0;
  let commands = 0;
  let others = 0;
  for (const tool of tools) {
    const name = String(tool.toolName || "").toLowerCase();
    if (/(grep|search|find|lookup|query)/.test(name)) searches += 1;
    else if (/(bash|shell|exec|run|command|terminal|spawn|process)/.test(name)) commands += 1;
    else if (/(read|file|content|cat|view|open|glob|list|ls|tree|fetch|get|inspect|diff)/.test(name)) files += 1;
    else others += 1;
  }
  const segs = [];
  if (files) segs.push(t(files === 1 ? "tool.runFile" : "tool.runFiles", { n: files }));
  if (searches) segs.push(t(searches === 1 ? "tool.runSearch" : "tool.runSearches", { n: searches }));
  if (commands) segs.push(t(commands === 1 ? "tool.runCommand" : "tool.runCommands", { n: commands }));
  if (others) segs.push(t(others === 1 ? "tool.runOther" : "tool.runOthers", { n: others }));
  const text = segs.join(", ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/* Renders a run of tool calls. <= TOOL_RUN_COLLAPSE_AFTER → one row each.
   More than that → a collapsed summary line that expands to the full rows. */
export function ToolRun({ tools }) {
  const t = useT();
  const hasError = tools.some((tool) => tool.toolStatus === "error");
  const [expanded, setExpanded] = React.useState(hasError);
  React.useEffect(() => {
    if (hasError) setExpanded(true);
  }, [hasError]);

  if (tools.length <= TOOL_RUN_COLLAPSE_AFTER) {
    return html`
      <div className="flex flex-col gap-3">
        ${tools.map(
          (tool, index) => html`<${ToolActivity}
            key=${tool.id || tool.callId || `${tool.toolName}-${index}`}
            activity=${tool}
          />`
        )}
      </div>
    `;
  }

  const summary = summarizeTools(t, tools);

  return html`
    <div className="flex flex-col">
      <button
        type="button"
        onClick=${() => setExpanded((value) => !value)}
        aria-expanded=${expanded ? "true" : "false"}
        className=${[
          "v2-button flex w-full items-center gap-2 border-0 bg-transparent px-1 py-1.5 text-left text-sm",
          hasError ? "text-[var(--v2-danger-text)]" : "text-iron-400 hover:text-iron-200",
        ].join(" ")}
      >
        <${Icon} name="layers" className="h-4 w-4 shrink-0" />
        <span className="truncate">${summary}</span>
        <${Icon}
          name="chevron"
          className=${["ml-auto h-3.5 w-3.5 shrink-0", expanded ? "rotate-180" : ""].join(" ")}
        />
      </button>

      ${expanded &&
      html`
        <div className="mt-2 flex flex-col gap-3">
          ${tools.map(
            (tool, index) => html`<${ToolActivity}
              key=${tool.id || tool.callId || `${tool.toolName}-${index}`}
              activity=${tool}
            />`
          )}
        </div>
      `}
    </div>
  `;
}

function ToolActivityCard({ activity, nested = false }) {
  const {
    toolName,
    toolStatus,
    toolDetail,
    toolError,
    toolDurationMs,
    toolParameters,
    toolResultPreview,
  } = activity;

  const [expanded, setExpanded] = React.useState(toolStatus === "error");
  React.useEffect(() => {
    if (toolStatus === "error") setExpanded(true);
  }, [toolStatus]);

  const dotClass = DOT_STYLE[toolStatus] || DOT_STYLE.running;
  const hasDuration = toolDurationMs !== null && toolDurationMs !== undefined;
  const controlsId = React.useId();

  const row = html`
    <button
      type="button"
      onClick=${() => setExpanded((v) => !v)}
      aria-expanded=${expanded ? "true" : "false"}
      aria-controls=${controlsId}
      className="v2-button flex w-full items-center gap-2.5 border-0 border-b border-iron-700/40 bg-transparent px-1 py-2 text-left text-sm"
    >
      <span className=${["h-2 w-2 shrink-0 rounded-full", dotClass].join(" ")} />
      <span className="shrink-0 font-mono text-[11px] uppercase tracking-wide text-iron-300"
        >${STATUS_WORD[toolStatus] || "run"}</span
      >
      <span className="shrink-0 truncate font-mono text-[13px] font-medium text-iron-100"
        >${toolName}</span
      >
      ${toolDetail &&
      html`<span className="min-w-0 truncate font-mono text-xs text-iron-400"
        >${toolDetail}</span
      >`}
      <span className="ml-auto flex shrink-0 items-center gap-2">
        ${hasDuration &&
        html`<span className="font-mono text-[11px] text-iron-300">${toolDurationMs}ms</span>`}
        <${Icon}
          name="chevron"
          className=${["h-3.5 w-3.5 text-iron-400", expanded ? "rotate-180" : ""].join(" ")}
        />
      </span>
    </button>
  `;

  return html`
    <div className=${nested ? "" : "flex gap-3"}>
      ${!nested &&
      html`
        <div
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-white/10 bg-iron-800 text-iron-100"
        >
          <${Icon} name="tool" className="h-4 w-4" />
        </div>
      `}
      <div className=${nested ? "min-w-0 flex-1" : "min-w-0 max-w-[85%] flex-1"}>
        ${row}
        ${expanded &&
        html`<${ToolDetailPanel}
          controlsId=${controlsId}
          toolDetail=${toolDetail}
          toolParameters=${toolParameters}
          toolResultPreview=${toolResultPreview}
          toolError=${toolError}
          toolStatus=${toolStatus}
          toolDurationMs=${hasDuration ? toolDurationMs : null}
        />`}
      </div>
    </div>
  `;
}

/* Tabbed Panel — Details / Parameters / Result / Error. Only tabs that have
   content are shown; the first available tab is selected by default (Error
   first when present so failures surface immediately). */
function ToolDetailPanel({
  controlsId,
  toolDetail,
  toolParameters,
  toolResultPreview,
  toolError,
  toolStatus,
  toolDurationMs,
}) {
  const t = useT();
  const tabs = React.useMemo(() => {
    const next = [];
    if (toolError) next.push({ id: "error", label: t("tool.tabError") });
    if (toolDetail) next.push({ id: "details", label: t("tool.tabDetails") });
    if (toolParameters) next.push({ id: "params", label: t("tool.tabParameters") });
    if (toolResultPreview) next.push({ id: "result", label: t("tool.tabResult") });
    return next;
  }, [t, toolError, toolDetail, toolParameters, toolResultPreview]);

  const [activeState, setActive] = React.useState(null);
  const active = activeState && tabs.some((tab) => tab.id === activeState)
    ? activeState
    : tabs[0]?.id;
  React.useEffect(() => {
    if (toolError) setActive("error");
  }, [toolError]);

  if (tabs.length === 0) {
    return html`
      <div
        id=${controlsId}
        className="rounded-b-lg border-x border-b border-iron-700/40 bg-iron-950 px-3 py-2 font-mono text-xs text-iron-400"
      >
        ${t("tool.noDetail")}
      </div>
    `;
  }

  return html`
    <div
      id=${controlsId}
      className="rounded-b-lg border-x border-b border-iron-700/40 bg-iron-950"
    >
      <div className="flex items-center gap-1 border-b border-iron-700/40 px-2 pt-1.5">
        ${tabs.map(
          (tab) => html`
            <button
              type="button"
              key=${tab.id}
              onClick=${() => setActive(tab.id)}
              className=${[
                "v2-button rounded-t-md px-2.5 py-1 font-mono text-[11px]",
                active === tab.id
                  ? "bg-iron-900 text-iron-100"
                  : "text-iron-400 hover:text-iron-200",
              ].join(" ")}
            >
              ${tab.label}
            </button>
          `
        )}
        <span className="ml-auto px-1 py-1 font-mono text-[10px] text-iron-500">
          ${toolStatus === "error"
            ? t("tool.exitError")
            : toolStatus === "running"
            ? t("tool.exitRunning")
            : t("tool.exitOk")}${toolDurationMs !== null ? ` · ${toolDurationMs}ms` : ""}
        </span>
      </div>
      <div className="p-3 text-xs">
        ${active === "details" &&
        html`<div className="whitespace-pre-wrap text-iron-200">${toolDetail}</div>`}
        ${active === "params" &&
        html`<pre className="overflow-x-auto rounded bg-iron-900 p-2 font-mono text-iron-100">${toolParameters}</pre>`}
        ${active === "result" && html`<${ToolResult} text=${toolResultPreview} />`}
        ${active === "error" &&
        html`<pre className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-danger-text)]">${toolError}</pre>`}
      </div>
    </div>
  `;
}

/* Rich tool-result rendering: inline image for data URLs, a table for arrays
   of flat objects, pretty-printed JSON for other structured payloads, and a
   plain preformatted block otherwise. */
function ToolResult({ text }) {
  const value = typeof text === "string" ? text.trim() : "";

  if (/^data:image\/(?:png|jpe?g|gif|webp|bmp);/i.test(value)) {
    return html`<img
      src=${value}
      alt="Tool result"
      className="max-h-72 rounded-lg border border-iron-700 object-contain"
    />`;
  }

  let parsed;
  if ((value.startsWith("{") || value.startsWith("[")) && value.length < 200000) {
    try {
      parsed = JSON.parse(value);
    } catch {
      parsed = undefined;
    }
  }

  if (Array.isArray(parsed) && parsed.length > 0 && parsed.every(isFlatRow)) {
    const columns = Array.from(
      parsed.reduce((set, row) => {
        Object.keys(row).forEach((k) => set.add(k));
        return set;
      }, new Set())
    );
    return html`
      <div className="overflow-x-auto rounded border border-iron-700/60">
        <table className="w-full border-collapse text-left font-mono text-[11px]">
          <thead>
            <tr>
              ${columns.map(
                (col) => html`<th
                  key=${col}
                  className="border-b border-iron-700/60 bg-iron-900 px-2 py-1 font-semibold text-iron-100"
                >${col}</th>`
              )}
            </tr>
          </thead>
          <tbody>
            ${parsed.map(
              (row, i) => html`<tr key=${i}>
                ${columns.map(
                  (col) => html`<td
                    key=${col}
                    className="border-b border-iron-700/40 px-2 py-1 text-iron-200"
                  >${formatCell(row[col])}</td>`
                )}
              </tr>`
            )}
          </tbody>
        </table>
      </div>
    `;
  }

  if (parsed !== undefined && typeof parsed === "object") {
    return html`<pre
      className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
    >${JSON.stringify(parsed, null, 2)}</pre>`;
  }

  return html`<pre
    className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]"
  >${text}</pre>`;
}

function isFlatRow(row) {
  return (
    row &&
    typeof row === "object" &&
    !Array.isArray(row) &&
    Object.values(row).every((v) => v === null || typeof v !== "object")
  );
}

function formatCell(value) {
  if (value === null || value === undefined) return "";
  return String(value);
}
