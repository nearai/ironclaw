import { useQuery } from "@tanstack/react-query";
import { fetchBudgetSettings } from "../lib/settings-api.js";

export function useBudget() {
  const query = useQuery({
    queryKey: ["settings-usage"],
    queryFn: fetchBudgetSettings,
    refetchOnWindowFocus: true,
    staleTime: 30_000,
  });

  return {
    budget: query.data || null,
    query,
  };
}
