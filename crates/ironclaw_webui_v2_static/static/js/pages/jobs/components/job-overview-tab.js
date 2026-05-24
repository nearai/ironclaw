import { html } from "../../../lib/html.js";
import { EmptyPanel, FlowList, Panel, StatusPill } from "../../../design-system/primitives.js";
import { MarkdownRenderer } from "../../chat/components/markdown-renderer.js";
import {
  formatDuration,
  formatJobDate,
  stateLabel,
  statusToneForState,
} from "../lib/jobs-presenters.js";

function MetaItem({ label, value }) {
  return html`
    <div className="border-t border-white/10 py-4">
      <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">${label}</div>
      <div className="mt-2 text-sm leading-6 text-white">${value || "Not available"}</div>
    </div>
  `;
}

export function JobOverviewTab({ job }) {
  const transitions = (job.transitions || []).map((transition) => ({
    title: `${stateLabel(transition.from)} -> ${stateLabel(transition.to)}`,
    description: [formatJobDate(transition.timestamp), transition.reason].filter(Boolean).join(" / "),
  }));

  return html`
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.2fr)_minmax(320px,0.8fr)]">
      <${Panel} className="p-5 sm:p-6">
        <div className="flex items-center justify-between gap-4">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Execution context</div>
            <h3 className="mt-2 text-xl font-semibold text-white">Timing, state, and runtime shape</h3>
          </div>
          <${StatusPill} tone=${statusToneForState(job.state)} label=${stateLabel(job.state)} />
        </div>

        <div className="mt-5 grid gap-x-6 md:grid-cols-2">
          <${MetaItem} label="Created" value=${formatJobDate(job.created_at)} />
          <${MetaItem} label="Started" value=${formatJobDate(job.started_at)} />
          <${MetaItem} label="Completed" value=${formatJobDate(job.completed_at)} />
          <${MetaItem} label="Duration" value=${formatDuration(job.elapsed_secs)} />
          <${MetaItem} label="Kind" value=${job.job_kind ? `${job.job_kind} job` : null} />
          <${MetaItem} label="Mode" value=${job.job_mode || "Default worker"} />
        </div>
      <//>

      <div className="space-y-5">
        <${Panel} className="p-5 sm:p-6">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Description</div>
          <h3 className="mt-2 text-xl font-semibold text-white">Mission brief</h3>
          ${job.description
            ? html`<${MarkdownRenderer} content=${job.description} className="mt-4 text-sm leading-7 text-iron-200" />`
            : html`<p className="mt-4 text-sm leading-6 text-iron-300">This job did not record a long-form description.</p>`}
        <//>

        ${transitions.length
          ? html`
              <${Panel} className="p-5 sm:p-6">
                <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Transitions</div>
                <h3 className="mt-2 text-xl font-semibold text-white">State timeline</h3>
                <div className="mt-3">
                  <${FlowList} items=${transitions} />
                </div>
              <//>
            `
          : html`
              <${EmptyPanel}
                title="No state history yet"
                description="Transitions appear here once the job advances or records a recovery event."
              />
            `}
      </div>
    </div>
  `;
}
