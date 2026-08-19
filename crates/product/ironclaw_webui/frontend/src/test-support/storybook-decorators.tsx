import type { Decorator } from "@storybook/react-vite";
import { useEffect, useState } from "react";
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

/**
 * A single stubbed HTTP route for {@link withStubbedFetch}. `match` is a
 * substring tested against the request URL; `method` defaults to `GET`.
 * `json` is the response body (a value, or a factory called per request so
 * time-sensitive fields like `expires_at` stay fresh); omit it for an empty
 * body. `status` defaults to 200 (or 204 when there is no body).
 */
export type FetchStubRoute = {
  match: string;
  method?: string;
  status?: number;
  json?: unknown | (() => unknown);
};

type StubbedFetch = typeof window.fetch & {
  __storybookStub?: true;
  __original?: typeof window.fetch;
};

/**
 * Stub `window.fetch` for the story lifetime so components that fetch
 * imperatively on mount (not through a react-query cache `withQueryClient` can
 * seed) receive deterministic responses instead of hitting a real backend — a
 * shared/deployed Storybook must never perform a real side effect (e.g. minting
 * a live pairing code). Matched routes return a JSON `Response`; anything
 * unmatched passes through to the real `fetch`. The original is restored on
 * unmount.
 */
export function withStubbedFetch(routes: FetchStubRoute[]): Decorator {
  return function StubbedFetchDecorator(Story) {
    // Install synchronously during render so the story's mount effects already
    // see the stub. Idempotent under React's double-invoked render.
    if (typeof window !== "undefined" && !(window.fetch as StubbedFetch).__storybookStub) {
      const original = window.fetch.bind(window);
      const stub = (async (input: RequestInfo | URL, init?: RequestInit) => {
        const url =
          typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
        const method = (init?.method ?? "GET").toUpperCase();
        const route = routes.find(
          (r) => (r.method ?? "GET").toUpperCase() === method && url.includes(r.match),
        );
        if (!route) return original(input, init);
        const value = typeof route.json === "function" ? (route.json as () => unknown)() : route.json;
        const hasBody = value !== undefined;
        return new Response(hasBody ? JSON.stringify(value) : null, {
          status: route.status ?? (hasBody ? 200 : 204),
          headers: { "Content-Type": "application/json" },
        });
      }) as StubbedFetch;
      stub.__storybookStub = true;
      stub.__original = original;
      window.fetch = stub;
    }
    useEffect(() => {
      return () => {
        const current = window.fetch as StubbedFetch;
        if (current.__storybookStub && current.__original) {
          window.fetch = current.__original;
        }
      };
    }, []);
    return <Story />;
  };
}
