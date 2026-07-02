import { html } from "../../../lib/html.js";
import { Panel, StatusPill } from "../../../design-system/primitives.js";

function attentionTone(item) {
  return item?.type === "failure" ? "danger" : "warning";
}

function attentionLabel(item) {
  return item?.type === "failure" ? "failure" : "gate";
}

export function ProjectsAttentionStrip({ items, onOpenItem }) {
  if (!items?.length) return null;

  return html`
    <${Panel} className="overflow-hidden border-amber-300/10 p-0">
      <div className="border-b border-amber-300/10 px-5 py-4 sm:px-6">
        <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-copper">Needs attention</div>
        <p className="mt-2 max-w-[70ch] text-sm leading-6 text-iron-200">
          Operator-visible gates and recent failures across your project workspace.
        </p>
      </div>
      <div className="grid gap-3 p-4 sm:p-5 xl:grid-cols-2">
        ${items.map((item) => html`
          <button
            key=${`${item.project_id}-${item.thread_id || item.message}`}
            onClick=${() => onOpenItem(item)}
            className="group rounded-2xl border border-white/10 bg-iron-950/55 p-4 text-left hover:border-signal/30 hover:bg-white/[0.05]"
          >
            <div className="flex items-start justify-between gap-3">
              <div>
                <div className="text-sm font-semibold text-white">${item.project_name}</div>
                <div className="mt-1 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
                  ${item.thread_id ? `Thread ${String(item.thread_id).slice(0, 8)}` : "Project"}
                </div>
              </div>
              <${StatusPill} tone=${attentionTone(item)} label=${attentionLabel(item)} />
            </div>
            <p className="mt-3 text-sm leading-6 text-iron-200">${item.message}</p>
            <div className="mt-4 text-xs uppercase tracking-[0.16em] text-signal group-hover:text-white">
              Open project
            </div>
          </button>
        `)}
      </div>
    <//>
  `;
}
