import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { authorizeTraceHold, fetchTraceCredits } from "../lib/settings-api";

export function useTraceCredits() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["trace-credits"],
    queryFn: fetchTraceCredits,
    // Credits change slowly (capture -> score gate -> submit -> server
    // accept). The server-side credit view is now memoized by the on-disk
    // input signature (see `scoped_credit_view`), so a poll on an unchanged
    // history is a couple of `stat`s, not an O(total submissions) rebuild.
    // Keep the surfaces live via a focus refetch (immediate when the user
    // returns) plus an infrequent interval, dedupe redundant focus refetches
    // with staleTime, and never poll while the tab is hidden; a mutation
    // (authorize) still invalidates immediately for prompt updates.
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
