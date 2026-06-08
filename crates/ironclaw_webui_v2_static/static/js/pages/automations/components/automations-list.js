import { useNavigate } from "react-router";
import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { EmptyPanel, Panel, StatusPill } from "../../../design-system/primitives.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { cn } from "../../../utils/cn.js";
import { filterAutomations } from "../lib/automations-presenters.js";

const AUTOMATION_FILTERS = [
  { value: "all", labelKey: "automations.filter.all" },
  { value: "active", labelKey: "automations.filter.active" },
  { value: "running", labelKey: "automations.filter.running" },
  { value: "failures", labelKey: "automations.filter.failures" },
  { value: "paused", labelKey: "automations.filter.paused" },
];

function MetaItem({ label, value, tone }) {
  return html`
    <div className="min-w-0 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-3">
      <div className="font-mono text-[10px] uppercase tracking-[0.14em] text-iron-400">
        ${label}
      </div>
      <div
        className=${cn(
          "mt-2 min-w-0 break-words text-sm text-iron-100",
          tone === "success" && "text-emerald-200",
          tone === "danger" && "text-red-200",
          tone === "info" && "text-sky-200"
        )}
      >
        ${value || "—"}
      </div>
    </div>
  `;
}

function RunDots({ runs }) {
  const t = useT();
  const visibleRuns = runs.slice(0, 8);
  if (!visibleRuns.length) {
    return html`<span className="text-xs text-iron-400">${t("automations.table.noRuns")}</span>`;
  }

  return html`
    <div className="flex items-center gap-1.5" aria-label=${t("automations.table.recentRuns")}>
      ${visibleRuns.map((run) => html`
        <span
          key=${recentRunKey(run)}
          title=${`${run.status_label} · ${run.fired_label}`}
          className=${cn(
            "h-3 w-3 rounded-full border",
            run.status === "ok" && "border-emerald-300/50 bg-emerald-400",
            run.status === "error" && "border-red-300/50 bg-red-400",
            run.status === "running" && "border-sky-300/60 bg-sky-400",
            run.status === "unknown" && "border-iron-500 bg-iron-600"
          )}
        />
      `)}
    </div>
  `;
}

function recentRunKey(run) {
  return run.run_id || run.thread_id || run.submitted_at || run.timestamp_source;
}

function automationRowKeyDown(event, automationId, onSelectAutomation) {
  if (event.key !== "Enter" && event.key !== " ") return;
  event.preventDefault();
  onSelectAutomation(automationId);
}

function RecentRunRow({ run, onOpenRun }) {
  const t = useT();
  const canOpen = Boolean(run.chat_path);

  return html`
    <div className="grid gap-3 border-b border-[var(--v2-panel-border)] py-3 last:border-0 sm:grid-cols-[6.5rem_minmax(0,1fr)_auto] sm:items-center">
      <div>
        <${StatusPill} tone=${run.status_tone} label=${run.status_label} />
      </div>
      <div className="min-w-0">
        <div className="text-sm font-semibold text-iron-100">${run.fired_label}</div>
        <div className="mt-1 truncate font-mono text-[11px] text-iron-400">
          ${run.thread_id
            ? `${t("automations.detail.thread")} ${run.thread_id}`
            : t("automations.detail.noThread")}
        </div>
        ${run.run_id &&
        html`
          <div className="mt-1 truncate font-mono text-[11px] text-iron-500">
            ${t("automations.detail.run")} ${run.run_id}
          </div>
        `}
      </div>
      <${Button}
        variant="secondary"
        size="sm"
        disabled=${!canOpen}
        onClick=${canOpen ? () => onOpenRun(run.chat_path) : undefined}
      >
        <${Icon} name="chat" className="h-4 w-4" />
        ${t("automations.detail.openRun")}
      <//>
    </div>
  `;
}

function AutomationDetailPanel({ automation }) {
  const t = useT();
  const navigate = useNavigate();

  if (!automation) {
    return html`
      <${Panel} className="p-4 sm:p-5">
        <${EmptyPanel}
          boxed=${false}
          title=${t("automations.detail.emptyTitle")}
          description=${t("automations.detail.emptyDescription")}
        />
      <//>
    `;
  }

  const activeRun = automation.current_run;

  return html`
    <${Panel} className="overflow-hidden">
      <div className="border-b border-[var(--v2-panel-border)] p-4 sm:p-5">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
          <div className="min-w-0">
            <h3 className="truncate text-xl font-semibold tracking-tight text-iron-100">
              ${automation.display_name}
            </h3>
            <div className="mt-2 truncate font-mono text-[11px] uppercase tracking-[0.12em] text-iron-400">
              ${automation.automation_id}
            </div>
          </div>
          <${StatusPill}
            tone=${automation.has_running_run ? "info" : automation.state_tone}
            label=${automation.has_running_run
              ? t("automations.status.running")
              : automation.state_label}
          />
        </div>
      </div>

      <div className="space-y-5 p-4 sm:p-5">
        <div className="grid gap-3 sm:grid-cols-2">
          <${MetaItem} label=${t("automations.detail.schedule")} value=${automation.schedule_label} />
          <${MetaItem}
            label=${t("automations.detail.successRate")}
            value=${automation.success_rate_label}
            tone=${automation.has_failed_runs ? "danger" : "success"}
          />
          <${MetaItem} label=${t("automations.detail.lastCompleted")} value=${automation.last_run_label} />
          <${MetaItem}
            label=${t("automations.detail.currentRun")}
            value=${activeRun?.run_id || activeRun?.thread_id || t("automations.detail.noCurrentRun")}
            tone=${automation.has_running_run ? "info" : null}
          />
        </div>

        <div>
          <div className="mb-2 flex items-center justify-between gap-3">
            <h4 className="text-sm font-semibold text-iron-100">
              ${t("automations.detail.recentRuns")}
            </h4>
            <${RunDots} runs=${automation.recent_runs} />
          </div>

          ${automation.recent_runs.length
            ? html`
                <div>
                  ${automation.recent_runs.map((run) => html`
                    <${RecentRunRow}
                      key=${recentRunKey(run)}
                      run=${run}
                      onOpenRun=${navigate}
                    />
                  `)}
                </div>
              `
            : html`
                <div className="rounded-xl border border-dashed border-[var(--v2-panel-border)] p-4 text-sm text-iron-300">
                  ${t("automations.detail.noRuns")}
                </div>
              `}
        </div>
      </div>
    <//>
  `;
}

export function AutomationsList({
  automations,
  filter,
  onFilterChange,
  onRefresh,
  isRefreshing,
  selectedAutomationId,
  onSelectAutomation,
}) {
  const t = useT();
  const filtered = filterAutomations(automations, filter);
  const hasAutomations = automations.length > 0;
  const selectedAutomation =
    filtered.find((automation) => automation.automation_id === selectedAutomationId) ||
    filtered[0] ||
    null;

  return html`
    <div className="space-y-5">
      <${Panel} className="p-4 sm:p-5">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">
              ${t("automations.eyebrow")}
            </div>
            <h2 className="mt-2 text-2xl font-semibold tracking-tight text-iron-100">
              ${t("automations.title")}
            </h2>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-iron-300">
              ${t("automations.description")}
            </p>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <div
              className="inline-flex overflow-hidden rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]"
              role="group"
              aria-label=${t("automations.filterLabel")}
            >
              ${AUTOMATION_FILTERS.map((item) => html`
                <button
                  key=${item.value}
                  type="button"
                  aria-pressed=${filter === item.value}
                  onClick=${() => onFilterChange(item.value)}
                  className=${cn(
                    "h-9 px-3 text-xs font-semibold",
                    filter === item.value
                      ? "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
                      : "text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
                  )}
                >
                  ${t(item.labelKey)}
                </button>
              `)}
            </div>
            <${Button}
              variant="secondary"
              size="icon-sm"
              aria-label=${t("automations.refresh")}
              disabled=${isRefreshing}
              onClick=${onRefresh}
            >
              <${Icon} name="retry" className="h-4 w-4" />
            <//>
          </div>
        </div>
      <//>

      ${!filtered.length
        ? html`
            <${EmptyPanel}
              title=${hasAutomations
                ? t("automations.empty.matchingTitle")
                : t("automations.empty.noneTitle")}
              description=${hasAutomations
                ? t("automations.empty.matchingDescription")
                : t("automations.empty.noneDescription")}
            />
          `
        : html`
            <div className="grid gap-5 xl:grid-cols-[minmax(0,1.12fr)_minmax(22rem,0.88fr)]">
              <${Panel} className="overflow-hidden">
                <div className="overflow-x-auto">
                  <table className="w-full min-w-[900px] border-collapse">
                    <thead>
                      <tr className="border-b border-[var(--v2-panel-border)] text-left">
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${t("automations.table.name")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${t("automations.table.schedule")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${t("automations.table.nextRun")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${t("automations.table.recentRuns")}
                        </th>
                        <th className="px-5 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-iron-300">
                          ${t("automations.table.status")}
                        </th>
                      </tr>
                    </thead>
                    <tbody>
                      ${filtered.map((automation) => {
                        const selected =
                          automation.automation_id === selectedAutomation?.automation_id;
                        return html`
                          <tr
                            key=${automation.automation_id}
                            tabIndex=${0}
                            role="button"
                            aria-pressed=${selected}
                            onClick=${() => onSelectAutomation(automation.automation_id)}
                            onKeyDown=${(event) =>
                              automationRowKeyDown(
                                event,
                                automation.automation_id,
                                onSelectAutomation
                              )}
                            className=${cn(
                              "cursor-pointer border-b border-[var(--v2-panel-border)] last:border-0 hover:bg-white/[0.03] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--v2-accent)]",
                              selected && "bg-[var(--v2-accent-soft)]/30"
                            )}
                          >
                            <td className="max-w-[280px] px-5 py-4 align-top">
                              <div className="truncate text-sm font-semibold text-iron-100">
                                ${automation.display_name}
                              </div>
                              <div className="mt-1 truncate font-mono text-[11px] uppercase tracking-[0.12em] text-iron-400">
                                ${automation.automation_id}
                              </div>
                            </td>
                            <td className="px-5 py-4 align-top text-sm text-iron-200">
                              ${automation.schedule_label}
                            </td>
                            <td className="px-5 py-4 align-top text-sm text-iron-200">
                              ${automation.next_run_label}
                            </td>
                            <td className="px-5 py-4 align-top">
                              <${RunDots} runs=${automation.recent_runs} />
                            </td>
                            <td className="px-5 py-4 align-top">
                              <${StatusPill}
                                tone=${automation.has_running_run
                                  ? "info"
                                  : automation.has_failed_runs
                                    ? "danger"
                                    : automation.state_tone}
                                label=${automation.has_running_run
                                  ? t("automations.status.running")
                                  : automation.has_failed_runs
                                    ? t("automations.status.needsReview")
                                    : automation.state_label}
                              />
                            </td>
                          </tr>
                        `;
                      })}
                    </tbody>
                  </table>
                </div>
              <//>

              <${AutomationDetailPanel} automation=${selectedAutomation} />
            </div>
          `}
    </div>
  `;
}
