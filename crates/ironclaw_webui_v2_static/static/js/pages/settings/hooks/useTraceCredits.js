import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { authorizeTraceHold, fetchTraceCredits } from "../lib/settings-api.js";

// Gateways without Trace Commons (e.g. the bundled desktop sidecar) return
// 403/404 for the credit endpoint. Treat that as "feature unavailable" — a calm
// empty state, not a red load-failure — and stop retrying/polling so the tab
// doesn't sit on an error banner and re-hit the missing route on every focus.
const TRACE_UNSUPPORTED_STATUSES = new Set([403, 404]);

export function useTraceCredits() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["trace-credits"],
    queryFn: fetchTraceCredits,
    retry: (count, error) =>
      !TRACE_UNSUPPORTED_STATUSES.has(error?.status) && count < 3,
    // Credits change slowly (capture -> score gate -> submit -> server
    // accept). The server-side credit view is now memoized by the on-disk
    // input signature (see `scoped_credit_view`), so a poll on an unchanged
    // history is a couple of `stat`s, not an O(total submissions) rebuild.
    // Keep the surfaces live via a focus refetch (immediate when the user
    // returns) plus an infrequent interval, dedupe redundant focus refetches
    // with staleTime, and never poll while the tab is hidden; a mutation
    // (authorize) still invalidates immediately for prompt updates. When the
    // gateway doesn't expose the endpoint, stop the interval entirely.
    refetchInterval: (q) =>
      TRACE_UNSUPPORTED_STATUSES.has(q?.state?.error?.status) ? false : 300_000,
    refetchIntervalInBackground: false,
    refetchOnWindowFocus: (q) =>
      !TRACE_UNSUPPORTED_STATUSES.has(q?.state?.error?.status),
    staleTime: 60_000,
  });
  const unsupported = TRACE_UNSUPPORTED_STATUSES.has(query.error?.status);

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
    unsupported,
  };
}
