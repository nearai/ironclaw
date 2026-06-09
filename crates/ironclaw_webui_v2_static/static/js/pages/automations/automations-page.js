import { React, html } from "../../lib/html.js";
import { useT } from "../../lib/i18n.js";
import { AutomationsList } from "./components/automations-list.js";
import { AutomationsSummaryStrip } from "./components/automations-summary-strip.js";
import { useAutomations } from "./hooks/useAutomations.js";

export function AutomationsPage() {
  const t = useT();
  const [filter, setFilter] = React.useState("all");
  const [selectedAutomationId, setSelectedAutomationId] = React.useState(null);
  const automationsState = useAutomations();
  const showErrorOnly =
    automationsState.error &&
    !automationsState.isLoading &&
    automationsState.automations.length === 0;

  React.useEffect(() => {
    if (!automationsState.automations.length) {
      setSelectedAutomationId(null);
      return;
    }
    const stillExists = automationsState.automations.some(
      (automation) => automation.automation_id === selectedAutomationId
    );
    if (!stillExists) {
      setSelectedAutomationId(automationsState.automations[0].automation_id);
    }
  }, [automationsState.automations, selectedAutomationId]);

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

          ${showErrorOnly
            ? null
            : html`
                <${AutomationsSummaryStrip} summary=${automationsState.summary} />

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
                        filter=${filter}
                        onFilterChange=${setFilter}
                        onRefresh=${automationsState.refetch}
                        isRefreshing=${automationsState.isRefreshing}
                        selectedAutomationId=${selectedAutomationId}
                        onSelectAutomation=${setSelectedAutomationId}
                      />
                    `}
              `}
        </div>
      </div>
    </div>
  `;
}
