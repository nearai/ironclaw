import type { Decorator } from "@storybook/react-vite";
import { useLayoutEffect, useRef, useState } from "react";
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
 * unmatched REJECTS by default, naming the URL — a story that forgets a route
 * fails loudly instead of quietly acquiring live network access. Pass
 * `{ passthrough: true }` for the rare story that genuinely needs the real
 * `fetch`; making that opt-in is what keeps "shared Storybook never reaches a
 * backend" a property of the harness rather than a convention each new story
 * has to remember.
 *
 * Each decorator instance OWNS its stub, and installs it from a layout effect
 * rather than during render. Two ordering hazards drive that shape:
 *
 * - A render can be abandoned (interrupted, suspended, or thrown out) without
 *   ever committing, and an abandoned render schedules no cleanup — a
 *   render-phase install would strand a stub on `window.fetch` forever.
 * - Switching stories mounts the incoming decorator while the outgoing one is
 *   still mounted, so a shared "is a stub installed?" guard would leave the new
 *   story running on the OLD story's routes and let the outgoing cleanup
 *   restore the real `fetch` underneath it — a live backend call from a story.
 *
 * `<Story />` is therefore withheld until the stub is live: the story's own
 * mount effects fetch imperatively, and they must never observe the real
 * `fetch`. The install runs in a layout effect, so the second render lands
 * synchronously before paint. Cleanup restores only while this instance still
 * owns the active stub, and the real `fetch` is carried forward via
 * `__original` so stubs never chain-wrap each other.
 */
export function withStubbedFetch(
  routes: FetchStubRoute[],
  options: { passthrough?: boolean } = {},
): Decorator {
  return function StubbedFetchDecorator(Story) {
    // Read through a ref so the installed stub always serves this render's
    // routes, even if the same instance is re-rendered with new ones.
    const routesRef = useRef(routes);
    routesRef.current = routes;
    const ownedRef = useRef<StubbedFetch | null>(null);
    const [installed, setInstalled] = useState(false);

    useLayoutEffect(() => {
      const current = window.fetch as StubbedFetch;
      const original =
        current.__storybookStub && current.__original
          ? current.__original
          : window.fetch.bind(window);
      const stub = (async (input: RequestInfo | URL, init?: RequestInit) => {
        const url =
          typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
        // `fetch(new Request(url, { method: "POST" }))` carries its method on
        // the Request, not on `init` — reading only `init` would classify it as
        // a GET and match the wrong route (or none).
        const requestMethod = input instanceof Request ? input.method : undefined;
        const method = (init?.method ?? requestMethod ?? "GET").toUpperCase();
        const route = routesRef.current.find(
          (r) => (r.method ?? "GET").toUpperCase() === method && url.includes(r.match),
        );
        if (!route) {
          if (options.passthrough) return original(input, init);
          throw new Error(
            `withStubbedFetch: unmatched ${method} ${url}. Stories must not reach a real ` +
              "backend — add a route for it, or pass { passthrough: true } if this story " +
              "genuinely needs the network.",
          );
        }
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
      setInstalled(true);
      return () => {
        // Only the instance that still owns the active stub may restore — an
        // incoming story that already installed its own must not be torn down.
        if (window.fetch === stub && stub.__original) {
          window.fetch = stub.__original;
        }
        ownedRef.current = null;
        setInstalled(false);
      };
    }, []);

    // Withheld until the stub is live, so a story that fetches on mount can
    // never observe the real `fetch`.
    if (!installed) return null;
    return <Story />;
  };
}
