import { useQuery } from "@tanstack/react-query";

import { fetchAutomations } from "../lib/automations-api.js";
import {
  automationSummary,
  normalizeAutomations,
} from "../lib/automations-presenters.js";

export function useAutomations() {
  const query = useQuery({
    queryKey: ["automations"],
    queryFn: fetchAutomations,
    refetchInterval: 5000,
  });

  const automations = normalizeAutomations(query.data);

  return {
    automations,
    summary: automationSummary(automations),
    isLoading: query.isLoading,
    isRefreshing: query.isFetching,
    error: query.error || null,
    refetch: query.refetch,
  };
}
