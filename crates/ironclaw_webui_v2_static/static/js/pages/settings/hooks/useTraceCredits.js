import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { authorizeTraceHold, fetchTraceCredits } from "../lib/settings-api.js";

export function useTraceCredits() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["trace-credits"],
    queryFn: fetchTraceCredits,
    // Credits change slowly (capture -> score gate -> submit -> server
    // accept). Each fetch rebuilds the full credit view server-side (reads and
    // rescans the entire local history + ships all holds), so an aggressive
    // poll turns an open tab into steady O(history) background work. Keep the
    // surfaces live via a focus refetch (immediate when the user returns) plus
    // an infrequent interval, dedupe redundant focus refetches with staleTime,
    // and never poll while the tab is hidden. A mutation (authorize) still
    // invalidates immediately for prompt updates.
    // TODO: incrementalize the server-side credit view so polling cost is
    // bounded by new submissions rather than total history.
    refetchInterval: 300_000,
    refetchIntervalInBackground: false,
    refetchOnWindowFocus: true,
    staleTime: 60_000,
  });

  // Authorize a held manual-review trace; on success the credits query
  // refetches so the held list and counts update without a manual reload.
  const authorize = useMutation({
    mutationFn: authorizeTraceHold,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["trace-credits"] }),
  });

  return {
    credits: query.data || null,
    query,
    authorize,
  };
}
