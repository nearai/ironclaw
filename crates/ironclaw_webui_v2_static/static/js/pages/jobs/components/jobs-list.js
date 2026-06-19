import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { Button } from "../../../design-system/button.js";
import { EmptyPanel, Panel, StatusPill } from "../../../design-system/primitives.js";
import {
  canShowCancel,
  formatJobDate,
  stateLabel,
  statusToneForState,
  truncateJobId,
} from "../lib/jobs-presenters.js";

export function JobsList({
  jobs,
  totalJobs,
  selectedJobId,
  search,
  onSearchChange,
  stateFilter,
  onStateFilterChange,
  onSelectJob,
  onCancelJob,
  isBusy,
  isRefreshing,
}) {
  const t = useT();
  const FILTERS = [
    { value: "all", label: t("jobs.list.filter.all") },
    { value: "pending", label: t("jobs.list.filter.pending") },
    { value: "in_progress", label: t("jobs.list.filter.inProgress") },
    { value: "completed", label: t("jobs.list.filter.completed") },
    { value: "failed", label: t("jobs.list.filter.failed") },
    { value: "stuck", label: t("jobs.list.filter.stuck") },
  ];

  if (!jobs.length) {
    const hasFilters = Boolean(search.trim()) || stateFilter !== "all";
    return html`
      <${EmptyPanel}
        title=${totalJobs && hasFilters ? t("jobs.list.empty.noMatchTitle") : t("jobs.list.empty.noJobsTitle")}
        description=${totalJobs && hasFilters
          ? t("jobs.list.empty.noMatchDesc")
          : t("jobs.list.empty.noJobsDesc")}
      />
    `;
  }

  return html`
    <div className="space-y-5">
      <${Panel} className="p-4 sm:p-5">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${t("jobs.list.explorer")}</div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-iron-100">${t("jobs.list.queueTitle")}</h2>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-iron-300">
              ${t("jobs.list.queueDesc")}
            </p>
          </div>
          <div className="flex items-center gap-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
            <span>${t("jobs.list.visible", { count: jobs.length })}</span>
            <span>/</span>
            <span>${isRefreshing ? t("jobs.list.state.refreshing") : t("jobs.list.state.live")}</span>
          </div>
        </div>

        <div className="mt-5 grid gap-3 md:grid-cols-[minmax(0,1fr)_220px]">
          <input
            value=${search}
            onInput=${(event) => onSearchChange(event.target.value)}
            placeholder=${t("jobs.list.searchPlaceholder")}
            className="h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          />
          <select
            value=${stateFilter}
            onChange=${(event) => onStateFilterChange(event.target.value)}
            className="v2-select h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          >
            ${FILTERS.map((filter) => html`<option key=${filter.value} value=${filter.value}>${filter.label}</option>`)}
          </select>
        </div>
      <//>

      <div className="grid gap-3">
        ${jobs.map((job) => html`
          <article
            key=${job.id}
            className=${[
              "group flex flex-col gap-4 rounded-[18px] border p-5",
              selectedJobId === job.id
                ? "border-signal/35 bg-signal/10"
                : "border-iron-700 bg-iron-800/60 hover:border-signal/30 hover:bg-iron-800/80",
            ].join(" ")}
          >
            <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
              <button onClick=${() => onSelectJob(job.id)} className="min-w-0 text-left">
                <div className="flex flex-wrap items-center gap-2">
                  <h3 className="truncate text-lg font-semibold text-iron-100">${job.title || t("jobs.list.untitled")}</h3>
                  <${StatusPill} tone=${statusToneForState(job.state)} label=${stateLabel(job.state)} />
                </div>
                <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
                  <span>${truncateJobId(job.id)}</span>
                  <span>${t("jobs.list.created", { value: formatJobDate(job.created_at) })}</span>
                  ${job.started_at && html`<span>${t("jobs.list.started", { value: formatJobDate(job.started_at) })}</span>`}
                </div>
              </button>

              <div className="flex gap-2">
                ${canShowCancel(job) && html`
                  <${Button}
                    variant="secondary"
                    className="h-9 px-3 text-xs"
                    disabled=${isBusy}
                    onClick=${() => onCancelJob(job.id)}
                  >
                    ${t("jobs.action.cancel")}
                  <//>
                `}
                <${Button} variant="ghost" className="h-9 px-3 text-xs" onClick=${() => onSelectJob(job.id)}>${t("jobs.action.open")}<//>
              </div>
            </div>
          </article>
        `)}
      </div>
    </div>
  `;
}
