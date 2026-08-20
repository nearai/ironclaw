import type { Decorator } from "@storybook/react-vite";
import { useEffect, useRef, useState } from "react";
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
  /** The real `window.fetch`, carried forward through every stub in a handoff. */
  __original?: typeof window.fetch;
};

/**
 * Stub `window.fetch` for the story lifetime so components that fetch
 * imperatively on mount (not through a react-query cache `withQueryClient` can
 * seed) receive deterministic responses instead of hitting a real backend — a
 * shared/deployed Storybook must never perform a real side effect (e.g. minting
 * a live pairing code). Matched routes return a JSON `Response`; anything
 * unmatched passes through to the real `fetch`.
 *
 * Each decorator instance OWNS its stub. Switching stories renders the incoming
 * decorator before React runs the outgoing one's cleanup, so a shared
 * "is a stub already installed?" guard would leave the new story running on the
 * *old* story's routes and would then let the outgoing cleanup restore the real
 * `fetch` underneath it — a live backend call from a story. Installing
 * unconditionally when `window.fetch` is not this instance's own stub, and
 * restoring only while this instance still owns the active stub, keeps the
 * handoff hermetic in both directions. The real `fetch` is carried forward via
 * `__original` so stubs never chain-wrap each other.
 */
export function withStubbedFetch(routes: FetchStubRoute[]): Decorator {
  return function StubbedFetchDecorator(Story) {
    // Read through a ref so the installed stub always serves this render's
    // routes, even if the same instance is re-rendered with new ones.
    const routesRef = useRef(routes);
    routesRef.current = routes;
    const ownedRef = useRef<StubbedFetch | null>(null);

    // Install synchronously during render so the story's mount effects already
    // see the stub. Idempotent under React's double-invoked render: the second
    // pass sees this instance's own stub already installed.
    if (typeof window !== "undefined" && window.fetch !== ownedRef.current) {
      const current = window.fetch as StubbedFetch;
      const original =
        current.__storybookStub && current.__original
          ? current.__original
          : window.fetch.bind(window);
      const stub = (async (input: RequestInfo | URL, init?: RequestInit) => {
        const url =
          typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
        const method = (init?.method ?? "GET").toUpperCase();
        const route = routesRef.current.find(
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
      ownedRef.current = stub;
      window.fetch = stub;
    }
    useEffect(() => {
      // Re-assert ownership: React's StrictMode runs cleanup once before the
      // real mount, and no render follows it to reinstall.
      const owned = ownedRef.current;
      if (owned && window.fetch !== owned) window.fetch = owned;
      return () => {
        const mine = ownedRef.current;
        // Only the instance that still owns the active stub may restore — an
        // incoming story that already installed its own must not be torn down.
        if (mine && window.fetch === mine && mine.__original) {
          window.fetch = mine.__original;
        }
      };
    }, []);
    return <Story />;
  };
}
