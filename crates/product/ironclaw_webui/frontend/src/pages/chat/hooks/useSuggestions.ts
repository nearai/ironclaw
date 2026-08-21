// @ts-nocheck
// OOBE suggestions data hook — the single owner of suggestion state in the
// browser (VISION-RECONCILIATION §5.1 slices 1-3).
//
// The backend owns durability, generation, and the suggestion -> thread/run
// binding; this hook only reads that state and issues the four mutations.
// Generation is never automatic: it costs a real model run, so `empty` renders
// a CTA and the user asks for it (discovery stays side-effect-free).
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  dismissSuggestion,
  fetchSuggestions,
  generateSuggestions,
  pollDelayMs,
  startSuggestion,
} from "../lib/suggestions-api";

export const SUGGESTIONS_QUERY_KEY = ["suggestions"];

export function useSuggestions() {
  const queryClient = useQueryClient();

  const query = useQuery({
    queryKey: SUGGESTIONS_QUERY_KEY,
    queryFn: ({ signal }) => fetchSuggestions({ signal }),
    // Poll only while the backend says work is in flight, at the cadence it
    // asks for. Any terminal status stops the timer — a constant interval
    // would keep hitting the route forever for a surface that is usually idle.
    refetchInterval: (q) =>
      q.state.data?.status === "generating" ? pollDelayMs(q.state.data) : false,
    refetchIntervalInBackground: false,
  });

  const generate = useMutation({
    mutationFn: () => generateSuggestions(),
    // The 202 body is the authoritative new state (status: generating), so
    // seed it directly rather than waiting for the next poll to catch up.
    onSuccess: (data) => {
      if (data) queryClient.setQueryData(SUGGESTIONS_QUERY_KEY, data);
    },
  });

  const start = useMutation({
    mutationFn: (suggestionId: string) => startSuggestion(suggestionId),
    // The started card gains a durable thread/run binding; refresh so it
    // renders as started if the user comes back to the landing.
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: SUGGESTIONS_QUERY_KEY });
    },
  });

  const dismiss = useMutation({
    mutationFn: (suggestionId: string) => dismissSuggestion(suggestionId),
    // Drop the card locally on success instead of refetching: the server has
    // already committed the dismissal, and a round-trip would leave the card
    // on screen for the duration.
    onSuccess: (_result, suggestionId) => {
      queryClient.setQueryData(SUGGESTIONS_QUERY_KEY, (previous) =>
        previous
          ? {
              ...previous,
              suggestions: (previous.suggestions || []).filter(
                (suggestion) => suggestion.id !== suggestionId,
              ),
            }
          : previous,
      );
    },
  });

  const data = query.data;

  return {
    // `isLoading` is distinct from `empty`: the first read hasn't resolved yet,
    // so the surface must not offer "generate" for a set that may already exist.
    isLoading: query.isLoading,
    status: data?.status ?? null,
    suggestions: data?.suggestions ?? [],
    loadError: query.error || null,
    generate: () => generate.mutate(),
    isGenerating: generate.isPending || data?.status === "generating",
    generateError: generate.error || null,
    start: (suggestionId, options) => start.mutate(suggestionId, options),
    startingId: start.isPending ? start.variables : null,
    startError: start.error || null,
    dismiss: (suggestionId) => dismiss.mutate(suggestionId),
  };
}
