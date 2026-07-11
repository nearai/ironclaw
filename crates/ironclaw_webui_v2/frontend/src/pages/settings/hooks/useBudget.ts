import { useQuery } from "@tanstack/react-query";
import { fetchBudgetSettings } from "../lib/settings-api";

// Read-only per-user usage/budget snapshot for the Usage settings tab. The
// server-side view is inexpensive to poll, so keep it live via a focus
// refetch plus a short staletime; a mutation elsewhere is not involved.
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
