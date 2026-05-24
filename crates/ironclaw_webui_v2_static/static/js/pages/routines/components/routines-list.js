import { EmptyPanel, Panel } from "../../../design-system/primitives.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { RoutineRow } from "./routine-row.js";

const FILTERS = [
  { value: "all", label: "All routines" },
  { value: "enabled", label: "Enabled" },
  { value: "disabled", label: "Disabled" },
  { value: "unverified", label: "Unverified" },
  { value: "failing", label: "Failing" },
];

export function RoutinesList({
  routines,
  totalRoutines,
  selectedRoutineId,
  search,
  onSearchChange,
  statusFilter,
  onStatusFilterChange,
  onSelectRoutine,
  onTriggerRoutine,
  onToggleRoutine,
  isBusy,
  isRefreshing,
}) {
  const t = useT();

  if (!routines.length) {
    const hasFilters = Boolean(search.trim()) || statusFilter !== "all";
    return html`
      <${EmptyPanel}
        title=${totalRoutines && hasFilters ? "No routines match" : "No routines yet"}
        description=${totalRoutines && hasFilters
          ? "Adjust the search or status filter to find a saved routine."
          : "Routines created from chat will appear here after they are saved."}
      />
    `;
  }

  return html`
    <div className="space-y-5">
      <${Panel} className="p-4 sm:p-5">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">
              ${t("routines.explorer")}
            </div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-iron-100">
              ${t("routines.title")}
            </h2>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-iron-300">
              ${t("routines.description")}
            </p>
          </div>
          <div className="flex items-center gap-2 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
            <span>${routines.length} visible</span>
            <span>/</span>
            <span>${isRefreshing ? "refreshing" : "live"}</span>
          </div>
        </div>

        <div className="mt-5 grid gap-3 md:grid-cols-[minmax(0,1fr)_220px]">
          <input
            value=${search}
            onInput=${(event) => onSearchChange(event.target.value)}
            placeholder="Search routine name, trigger, or action"
            className="h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          />
          <select
            value=${statusFilter}
            onChange=${(event) => onStatusFilterChange(event.target.value)}
            className="v2-select h-11 rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
          >
            ${FILTERS.map((filter) => html`<option key=${filter.value} value=${filter.value}>${filter.label}<//>`)}
          </select>
        </div>
      <//>

      <div className="grid gap-3">
        ${routines.map(
          (routine) => html`
            <${RoutineRow}
              key=${routine.id}
              routine=${routine}
              selectedRoutineId=${selectedRoutineId}
              onSelectRoutine=${onSelectRoutine}
              onTriggerRoutine=${onTriggerRoutine}
              onToggleRoutine=${onToggleRoutine}
              isBusy=${isBusy}
            />
          `
        )}
      </div>
    </div>
  `;
}
