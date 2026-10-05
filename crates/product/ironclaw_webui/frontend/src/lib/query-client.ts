import { QueryClient } from "@tanstack/react-query";

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // Refetch on focus so a returning user rebuilds fresh run/action state:
      // a backgrounded tab's SSE stream can miss a run's settle event, and the
      // default `false` left the stale tool activity on screen.
      refetchOnWindowFocus: true,
      retry: 1,
      staleTime: 10_000,
    },
  },
});
