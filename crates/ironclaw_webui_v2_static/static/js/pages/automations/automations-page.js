import { useParams } from "react-router";
import { React, html } from "../../lib/html.js";
import { useT } from "../../lib/i18n.js";
import { AutomationDetailPage } from "./components/automation-detail-page.js";
import { AutomationsList } from "./components/automations-list.js";
import { AutomationsSummaryStrip } from "./components/automations-summary-strip.js";
import { useAutomations } from "./hooks/useAutomations.js";
import { useOutboundDeliveryDefaults } from "./hooks/useOutboundDeliveryDefaults.js";

export function AutomationsPage() {
  const t = useT();
  const { automationId } = useParams();
  const [filter, setFilter] = React.useState("all");
  const automationsState = useAutomations();
  const deliveryState = useOutboundDeliveryDefaults();

  // A local refetch can resolve almost instantly, leaving the spinner to flash
  // imperceptibly. Hold a minimum spin window so a manual refresh always reads
  // as a deliberate action.
  const [minSpin, setMinSpin] = React.useState(false);
  const minSpinTimer = React.useRef(null);
  React.useEffect(() => () => clearTimeout(minSpinTimer.current), []);
  const handleRefresh = React.useCallback(() => {
    setMinSpin(true);
    clearTimeout(minSpinTimer.current);
    minSpinTimer.current = setTimeout(() => setMinSpin(false), 1000);
    automationsState.refetch();
  }, [automationsState.refetch]);
  const isRefreshing = automationsState.isRefreshing || minSpin;
  const showErrorOnly =
    automationsState.error &&
    !automationsState.isLoading &&
    automationsState.automations.length === 0;

  // Full-screen, persistent detail view for a single automation. Reached by
  // popping out of the list's detail modal or by deep link.
  if (automationId) {
    const automation =
      automationsState.automations.find(
        (item) => item.automation_id === automationId
      ) || null;
    return html`<${AutomationDetailPage}
      automation=${automation}
      isLoading=${automationsState.isLoading}
      error=${automationsState.error}
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
                <${AutomationsSummaryStrip}
                  summary=${automationsState.summary}
                  nextRunAt=${automationsState.nextRunAt}
                  activeFilter=${filter}
                  onSelectFilter=${setFilter}
                />

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
                        onRefresh=${handleRefresh}
                        isRefreshing=${isRefreshing}
                        deliveryState=${deliveryState}
                      />
                    `}
              `}
        </div>
      </div>
    </div>
  `;
}
