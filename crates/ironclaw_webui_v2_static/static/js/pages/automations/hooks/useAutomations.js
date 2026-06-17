import { useQuery } from "@tanstack/react-query";
import { React } from "../../../lib/html.js";
import { listAutomations } from "../../../lib/api.js";
import { useI18n } from "../../../lib/i18n.js";

import {
  automationSummary,
  normalizeAutomations,
} from "../lib/automations-presenters.js";

const AUTOMATIONS_PAGE_LIMIT = 50;
const AUTOMATION_RUNS_LIMIT = 25;

export function useAutomations() {
  const { t, lang } = useI18n();
  const query = useQuery({
    queryKey: ["automations"],
    queryFn: () =>
      listAutomations({
        limit: AUTOMATIONS_PAGE_LIMIT,
        runLimit: AUTOMATION_RUNS_LIMIT,
      }),
    refetchInterval: 30000,
    refetchIntervalInBackground: false,
  });

  // Schedule labels are localized in the presenter (`scheduleLabel`), so the
  // memo must re-run when the active language changes, not just the data.
  const automations = React.useMemo(
    () => normalizeAutomations(query.data, t, lang),
    [query.data, t, lang]
  );
  const summary = React.useMemo(
    () => automationSummary(automations),
    [automations]
  );

  // The scheduler (trigger poller) may be turned off, in which case listed
  // automations never fire. Treat an absent flag as enabled so we don't show a
  // false "off" notice against an older payload.
  const schedulerEnabled = query.data?.scheduler_enabled !== false;

  return {
    automations,
    summary,
    schedulerEnabled,
    isLoading: query.isLoading,
    isRefreshing: query.isFetching,
    error: query.error || null,
    refetch: query.refetch,
  };
}
