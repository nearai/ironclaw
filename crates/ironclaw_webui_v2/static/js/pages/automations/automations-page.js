import { useParams } from "react-router";
import { React, html } from "../../lib/html.js";
import { useT } from "../../lib/i18n.js";
import { AutomationDetailPage } from "./components/automation-detail-page.js";
import { AutomationsList } from "./components/automations-list.js";
import { useAutomation } from "./hooks/useAutomation.js";
import { useAutomations } from "./hooks/useAutomations.js";
import { useOutboundDeliveryDefaults } from "./hooks/useOutboundDeliveryDefaults.js";

export function AutomationsPage() {
  const t = useT();
  const { automationId } = useParams();
  const [filter, setFilter] = React.useState("all");
  const includeCompleted = filter === "completed";
  const automationsState = useAutomations(includeCompleted);
  const deliveryState = useOutboundDeliveryDefaults();

  // Detail view resolution. Hooks must run unconditionally, so `useAutomation`
  // is always called (it no-ops when there is no `automationId`). Seed it with
  // the list row when present so popping out of the list modal paints instantly;
  // the by-id fetch then resolves completed / beyond-the-page automations the
  // list cannot.
  const detailSeed = automationId
    ? automationsState.automations.find(
        (item) => item.automation_id === automationId
      ) || null
    : null;
  const detailState = useAutomation(automationId, { seed: detailSeed });

  // Data stays fresh on its own: a base poll plus a smart timer that pulls
  // near-due and in-progress automations forward (see useAutomations), so there
  // is no manual refresh control to reason about.
  const showErrorOnly =
    automationsState.error &&
    !automationsState.isLoading &&
    automationsState.automations.length === 0;

  // Full-screen, persistent detail view for a single automation. Reached by
  // popping out of the list's detail modal or by deep link.
  if (automationId) {
    return html`<${AutomationDetailPage}
      automation=${detailState.automation}
      isLoading=${detailState.isLoading}
      error=${detailState.error}
      isMutating=${automationsState.isMutating}
      onPauseAutomation=${automationsState.pauseAutomation}
      onResumeAutomation=${automationsState.resumeAutomation}
      onDeleteAutomation=${automationsState.deleteAutomation}
    />`;
  }

  return html`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          ${automationsState.error &&
          html`
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              ${t("automations.error.loadFailed")}
            </div>
          `}
          ${automationsState.actionError &&
          html`
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              ${automationsState.actionError.message}
            </div>
          `}

          ${showErrorOnly
            ? null
            : html`
                ${!automationsState.isLoading &&
                !automationsState.schedulerEnabled &&
                html`
                  <div
                    role="status"
                    className="rounded-xl border border-amber-400/30 bg-amber-500/10 px-4 py-3"
                  >
                    <div className="text-sm font-semibold text-amber-200">
                      ${t("automations.schedulerOff.title")}
                    </div>
                    <div className="mt-0.5 text-xs leading-5 text-amber-200/80">
                      ${t("automations.schedulerOff.description")}
                    </div>
                  </div>
                `}
                ${automationsState.isLoading
                  ? html`
                      <div className="space-y-4">
                        ${[1, 2, 3].map(
                          (index) =>
                            html`<div
                              key=${index}
                              className="v2-skeleton h-28 rounded-[18px]"
                            />`
                        )}
                      </div>
                    `
                  : html`
                      <${AutomationsList}
                        automations=${automationsState.automations}
                        summary=${automationsState.summary}
                        nextRunAt=${automationsState.nextRunAt}
                        filter=${filter}
                        onFilterChange=${setFilter}
                        deliveryState=${deliveryState}
                        isMutating=${automationsState.isMutating}
                        onPauseAutomation=${automationsState.pauseAutomation}
                        onResumeAutomation=${automationsState.resumeAutomation}
                        onDeleteAutomation=${automationsState.deleteAutomation}
                      />
                    `}
              `}
        </div>
      </div>
    </div>
  `;
}
