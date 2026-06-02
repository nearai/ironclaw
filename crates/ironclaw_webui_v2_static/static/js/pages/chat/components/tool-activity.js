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

export function ToolActivity({ activity }) {
  if (activity.toolCalls && activity.toolCalls.length > 0) {
    return html`<${ToolActivityGroup} toolCalls=${activity.toolCalls} />`;
  }
  return html`<${ToolActivityCard} activity=${activity} />`;
}

function ToolActivityGroup({ toolCalls }) {
  const hasError = toolCalls.some((tool) => tool.toolStatus === "error");
  const [expanded, setExpanded] = React.useState(hasError);
  React.useEffect(() => {
    if (hasError) setExpanded(true);
  }, [hasError]);
  const toolWord = toolCalls.length === 1 ? "tool" : "tools";

  return html`
    <div className="flex gap-3">
      <div
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-white/10 bg-iron-800 text-iron-100"
      >
        <${Icon} name="tool" className="h-4 w-4" />
      </div>
      <div className="min-w-0 max-w-[85%] flex-1">
        <button
          type="button"
          onClick=${() => setExpanded((value) => !value)}
          aria-expanded=${expanded ? "true" : "false"}
          className="v2-button flex w-full items-center gap-2 rounded-lg border-0 bg-transparent px-1 py-1.5 text-left text-sm text-iron-200"
        >
          <span
            className=${["h-2 w-2 shrink-0 rounded-full", hasError ? DOT_STYLE.error : DOT_STYLE.success].join(" ")}
          />
          <span className="font-medium text-iron-100"
            >Used ${toolCalls.length} ${toolWord}</span
          >
          <${Icon}
            name="chevron"
            className=${["ml-auto h-4 w-4 shrink-0", expanded ? "rotate-180" : ""].join(" ")}
          />
        </button>

        ${expanded &&
        html`
          <div className="mt-1 flex flex-col">
            ${toolCalls.map(
              (tool, index) => html`
                <${ToolActivityCard}
                  key=${tool.callId || `${tool.toolName}-${index}`}
                  activity=${tool}
                  nested=${true}
                />
              `
            )}
          </div>
        `}
      </div>
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
        ${toolStatus === "running" &&
        !hasDuration &&
        html`<span className="font-mono text-[11px] text-iron-300">…</span>`}
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
  const tabs = [];
  if (toolError) tabs.push({ id: "error", label: t("tool.tabError") });
  if (toolDetail) tabs.push({ id: "details", label: t("tool.tabDetails") });
  if (toolParameters) tabs.push({ id: "params", label: t("tool.tabParameters") });
  if (toolResultPreview) tabs.push({ id: "result", label: t("tool.tabResult") });

  const [active, setActive] = React.useState(tabs[0]?.id);
  React.useEffect(() => {
    if (tabs.length && !tabs.some((tab) => tab.id === active)) {
      setActive(tabs[0].id);
    }
  }, [tabs.map((tab) => tab.id).join(","), active]);

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
            : t("tool.exitOk")}${toolDurationMs !== null ? ` · ${toolDurationMs}ms` : ""}
        </span>
      </div>
      <div className="p-3 text-xs">
        ${active === "details" &&
        html`<div className="whitespace-pre-wrap text-iron-200">${toolDetail}</div>`}
        ${active === "params" &&
        html`<pre className="overflow-x-auto rounded bg-iron-900 p-2 font-mono text-iron-100">${toolParameters}</pre>`}
        ${active === "result" &&
        html`<pre className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-positive-text)]">${toolResultPreview}</pre>`}
        ${active === "error" &&
        html`<pre className="overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-[var(--v2-danger-text)]">${toolError}</pre>`}
      </div>
    </div>
  `;
}
