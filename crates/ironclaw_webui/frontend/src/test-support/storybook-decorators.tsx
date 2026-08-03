import type { Decorator } from "@storybook/react-vite";
import { useState } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router";

/**
 * Shared Storybook decorators for the app-wired `Components/*` stories.
 *
 * i18n is NOT provided here: `.storybook/preview.tsx` imports `src/i18n/en` as a
 * side effect, which populates the default `useT()` pack for every story. These
 * decorators only supply the router and react-query contexts that the shared
 * components read from.
 */

/** Wrap a story in a MemoryRouter so `NavLink` / `useNavigate` / `useLocation` work. */
export function withRouter(initialPath = "/chat"): Decorator {
  return (Story) => (
    <MemoryRouter initialEntries={[initialPath]}>
      <Story />
    </MemoryRouter>
  );
}

/**
 * Provide a fresh QueryClient per story mount. `seed` may pre-populate query
 * data (via `client.setQueryData`) so a component that reads a cached query
 * renders its loaded state without a network call — `staleTime: Infinity`
 * prevents a background refetch of the seeded entry.
 */
export function withQueryClient(seed?: (client: QueryClient) => void): Decorator {
  return function QueryClientDecorator(Story) {
    const [client] = useState(() => {
      const created = new QueryClient({
        defaultOptions: {
          queries: { retry: false, staleTime: Infinity, gcTime: Infinity },
        },
      });
      seed?.(created);
      return created;
    });
    return (
      <QueryClientProvider client={client}>
        <Story />
      </QueryClientProvider>
    );
  };
}
