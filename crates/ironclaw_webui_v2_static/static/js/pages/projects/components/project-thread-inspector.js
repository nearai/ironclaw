import { html } from "../../../lib/html.js";
import { Panel, StatusPill } from "../../../design-system/primitives.js";
import { MarkdownRenderer } from "../../chat/components/markdown-renderer.js";
import {
  formatCurrency,
  formatProjectDate,
  messageContent,
  threadPresentation,
  threadTone,
} from "../lib/projects-presenters.js";

function MetaCard({ label, value }) {
  return html`
    <div className="rounded-2xl border border-white/8 bg-iron-950/60 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${label}</div>
      <div className="mt-2 text-sm leading-6 text-white">${value}</div>
    </div>
  `;
}

export function ProjectThreadInspector({ thread }) {
  const presentation = threadPresentation(thread);

  return html`
    <div className="space-y-4">
      <${Panel} className="p-4 sm:p-5">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${presentation.subtitle}</div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-white">${presentation.title}</h2>
          </div>
          <${StatusPill} tone=${threadTone(thread.state)} label=${thread.state} />
        </div>

        ${presentation.brief
          ? html`
              <div className="mt-4 rounded-2xl border border-mint/15 bg-mint/10 p-4">
                <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-mint">Mission brief</div>
                <div className="mt-3 text-sm leading-6 text-iron-100">
                  <${MarkdownRenderer} content=${presentation.brief} />
                </div>
              </div>
            `
          : null}

        <div className="mt-5 grid gap-3 sm:grid-cols-2">
          <${MetaCard} label="Thread type" value=${thread.thread_type || "mission_run"} />
          <${MetaCard} label="Steps" value=${thread.step_count || 0} />
          <${MetaCard} label="Tokens" value=${(thread.total_tokens || 0).toLocaleString()} />
          <${MetaCard} label="Spend" value=${thread.total_cost_usd ? formatCurrency(thread.total_cost_usd) : "Not measured"} />
          <${MetaCard} label="Created" value=${formatProjectDate(thread.created_at)} />
          <${MetaCard} label="Completed" value=${thread.completed_at ? formatProjectDate(thread.completed_at) : "Still running"} />
        </div>
      <//>

      <${Panel} className="p-4 sm:p-5">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Timeline</div>
        <div className="mt-4 space-y-3">
          ${thread.messages?.length
            ? thread.messages.map((message, index) => html`
                <article key=${index} className="rounded-2xl border border-white/8 bg-iron-950/60 p-4">
                  <div className="text-xs uppercase tracking-[0.16em] text-iron-400">${message.role || "System"}</div>
                  <div className="mt-3 text-sm leading-6 text-iron-100">
                    <${MarkdownRenderer} content=${messageContent(message)} />
                  </div>
                </article>
              `)
            : html`
                <div className="rounded-2xl border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                  No messages were captured for this thread.
                </div>
              `}
        </div>
      <//>
    </div>
  `;
}
