import { useQuery } from "@tanstack/react-query";
import { React } from "../../../lib/html.js";

import { fetchAutomations } from "../lib/automations-api.js";
import {
  automationSummary,
  normalizeAutomations,
} from "../lib/automations-presenters.js";

export function useAutomations() {
  const query = useQuery({
    queryKey: ["automations"],
    queryFn: fetchAutomations,
    refetchInterval: 30000,
    refetchIntervalInBackground: false,
  });

  const automations = React.useMemo(
    () => normalizeAutomations(query.data),
    [query.data]
  );
  const summary = React.useMemo(
    () => automationSummary(automations),
    [automations]
  );

  return {
    automations,
    summary,
    isLoading: query.isLoading,
    isRefreshing: query.isFetching,
    error: query.error || null,
    refetch: query.refetch,
  };
}
