import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { authorizeTraceHold, fetchTraceCredits } from "../lib/settings-api.js";

export function useTraceCredits() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["trace-credits"],
    queryFn: fetchTraceCredits,
    // Credits change slowly (capture -> score gate -> submit -> server
    // accept), but the sidebar card and Settings tab should reflect new
    // accepted submissions without a manual reload. A 60s poll plus a
    // focus refetch is cheap and keeps both surfaces live.
    refetchInterval: 60_000,
    refetchOnWindowFocus: true,
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
