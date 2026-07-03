import { html } from "../../../lib/html.js";
import { Button } from "../../../design-system/button.js";
import { Panel, StatusPill } from "../../../design-system/primitives.js";
import { MarkdownRenderer } from "../../chat/components/markdown-renderer.js";
import { formatProjectDate, missionTone } from "../lib/projects-presenters.js";

function MetaCard({ label, value }) {
  return html`
    <div className="rounded-2xl border border-white/8 bg-iron-950/60 p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${label}</div>
      <div className="mt-2 text-sm leading-6 text-white">${value}</div>
    </div>
  `;
}

export function ProjectMissionInspector({
  mission,
  onFire,
  onPause,
  onResume,
  onOpenThread,
  isBusy,
}) {
  const actionButtons = [];
  if (mission.status === "Active") {
    actionButtons.push(html`<${Button} key="fire" onClick=${() => onFire(mission.id)} disabled=${isBusy}>Fire now<//>`);
    actionButtons.push(html`<${Button} key="pause" variant="secondary" onClick=${() => onPause(mission.id)} disabled=${isBusy}>Pause<//>`);
  } else if (mission.status === "Paused") {
    actionButtons.push(html`<${Button} key="resume" onClick=${() => onResume(mission.id)} disabled=${isBusy}>Resume<//>`);
    actionButtons.push(html`<${Button} key="fire" variant="secondary" onClick=${() => onFire(mission.id)} disabled=${isBusy}>Run once<//>`);
  } else {
    actionButtons.push(html`<${Button} key="retry" onClick=${() => onFire(mission.id)} disabled=${isBusy}>Run again<//>`);
  }

  return html`
    <div className="space-y-4">
      <${Panel} className="p-4 sm:p-5">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Mission dossier</div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-white">${mission.name}</h2>
          </div>
          <${StatusPill} tone=${missionTone(mission.status)} label=${mission.status} />
        </div>

        <div className="mt-4 grid gap-3 sm:grid-cols-2">
          <${MetaCard} label="Cadence" value=${mission.cadence_description || mission.cadence_type || "manual"} />
          <${MetaCard} label="Threads today" value=${`${mission.threads_today || 0} / ${mission.max_threads_per_day || "∞"}`} />
          <${MetaCard} label="Next fire" value=${mission.next_fire_at ? formatProjectDate(mission.next_fire_at) : "Not scheduled"} />
          <${MetaCard} label="Created" value=${formatProjectDate(mission.created_at)} />
        </div>

        <div className="mt-5 flex flex-wrap gap-2">${actionButtons}</div>
      <//>

      <${Panel} className="p-4 sm:p-5">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Mission brief</div>
        <div className="mt-4 text-sm leading-6 text-iron-200">
          <${MarkdownRenderer} content=${mission.goal || "No mission goal set."} />
        </div>
      <//>

      ${mission.current_focus
        ? html`
            <${Panel} className="p-4 sm:p-5">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Current focus</div>
              <div className="mt-4 text-sm leading-6 text-iron-200">
                <${MarkdownRenderer} content=${mission.current_focus} />
              </div>
            <//>
          `
        : null}

      ${mission.success_criteria
        ? html`
            <${Panel} className="p-4 sm:p-5">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Success criteria</div>
              <div className="mt-4 text-sm leading-6 text-iron-200">
                <${MarkdownRenderer} content=${mission.success_criteria} />
              </div>
            <//>
          `
        : null}

      ${mission.approach_history?.length
        ? html`
            <${Panel} className="p-4 sm:p-5">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Approach history</div>
              <div className="mt-4 space-y-3">
                ${mission.approach_history.map((entry, index) => html`
                  <div key=${index} className="rounded-2xl border border-white/8 bg-iron-950/60 p-4">
                    <div className="mb-3 text-xs uppercase tracking-[0.16em] text-iron-400">Run ${index + 1}</div>
                    <${MarkdownRenderer} content=${entry} />
                  </div>
                `)}
              </div>
            <//>
          `
        : null}

      ${mission.threads?.length
        ? html`
            <${Panel} className="p-4 sm:p-5">
              <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Spawned threads</div>
              <div className="mt-4 space-y-3">
                ${mission.threads.map((thread) => html`
                  <button
                    key=${thread.id}
                    onClick=${() => onOpenThread(thread.id)}
                    className="w-full rounded-2xl border border-white/8 bg-iron-950/60 p-4 text-left hover:border-signal/30 hover:bg-white/[0.05]"
                  >
                    <div className="flex items-center justify-between gap-3">
                      <div className="min-w-0 truncate text-sm font-semibold text-white">${thread.goal}</div>
                      <${StatusPill} tone=${missionTone(thread.state === "Running" ? "Active" : thread.state === "Failed" ? "Failed" : "Completed")} label=${thread.state} />
                    </div>
                  </button>
                `)}
              </div>
            <//>
          `
        : null}
    </div>
  `;
}
