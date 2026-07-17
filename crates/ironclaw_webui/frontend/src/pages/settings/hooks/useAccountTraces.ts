import { useQuery } from "@tanstack/react-query";
import { fetchAccountTraces } from "../lib/settings-api";

export function useAccountTraces() {
  const query = useQuery({
    queryKey: ["account-traces"],
    queryFn: fetchAccountTraces,
    // Trace submissions change slowly. Mirror the cadence used by
    // useTraceCredits: focus refetch for immediacy, infrequent poll to
    // stay live without hammering the server. staleTime dedupes redundant
    // focus refetches when the data is fresh.
    refetchInterval: 300_000,
    refetchIntervalInBackground: false,
    refetchOnWindowFocus: true,
    staleTime: 60_000,
  });
  return { traces: query.data?.traces || [], enrolled: !!query.data?.enrolled, query };
}
