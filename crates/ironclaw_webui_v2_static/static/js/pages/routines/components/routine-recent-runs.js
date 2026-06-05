import { StatusPill } from "../../../design-system/primitives.js";
import { html } from "../../../lib/html.js";
import { formatRoutineDate } from "../lib/routines-presenters.js";

function runTone(status) {
  if (status === "ok") return "success";
  if (status === "running") return "warning";
  return "danger";
}

export function RoutineRecentRuns({ runs }) {
  if (!runs?.length) {
    return html`
      <div className="rounded-xl border border-iron-700 bg-iron-950/40 p-4 text-sm text-iron-300">
        No runs recorded yet.
      </div>
    `;
  }

  return html`
    <div className="space-y-3">
      ${runs.map(
        (run) => html`
          <div key=${run.id} className="rounded-xl border border-iron-700 bg-iron-950/40 p-4">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <${StatusPill} tone=${runTone(run.status)} label=${run.status} />
              <span className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-400">
                ${formatRoutineDate(run.started_at)}
              </span>
            </div>
            ${run.result_summary &&
            html`<p className="mt-3 text-sm leading-6 text-iron-300">${run.result_summary}</p>`}
          </div>
        `
      )}
    </div>
  `;
}
