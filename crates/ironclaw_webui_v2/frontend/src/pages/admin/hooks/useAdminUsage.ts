import { useQuery } from "@tanstack/react-query";
import { fetchUsageSummary, fetchUsage } from "../lib/admin-api";

export function useUsageSummary() {
  return useQuery({
    queryKey: ["admin", "usage-summary"],
    queryFn: fetchUsageSummary,
    refetchInterval: 30_000,
  });
}

export function useUsage(period = "day", userId) {
  return useQuery({
    queryKey: ["admin", "usage", period, userId],
    queryFn: () => fetchUsage(period, userId),
    refetchInterval: 30_000,
  });
}
