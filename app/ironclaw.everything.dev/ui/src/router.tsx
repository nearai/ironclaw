import { dehydrate, hydrate } from "@tanstack/react-query";
import { createBrowserHistory, createRouter as createTanStackRouter } from "@tanstack/react-router";
import type { CreateRouterOptions } from "./app";
import { createAuthClient } from "./app";
import { routeTree } from "./routeTree.gen";

export type {
  ClientRuntimeConfig,
  CreateRouterOptions,
  RouterContext,
  RouterModule,
} from "./app";

export function createRouter(opts: CreateRouterOptions) {
  const queryClient = opts.context.queryClient;
  const history = opts.history ?? createBrowserHistory();
  const cspNonce = opts.context.cspNonce;

  const router = createTanStackRouter({
    routeTree,
    history,
    basepath: opts.basepath ?? opts.context.runtimeConfig?.runtime?.runtimeBasePath ?? "/",
    context: {
      queryClient,
      runtimeConfig: opts.context.runtimeConfig,
      cspNonce: opts.context.cspNonce,
      apiClient: opts.context.apiClient,
      authClient:
        opts.context.authClient ??
        createAuthClient({
          runtimeConfig: opts.context.runtimeConfig,
          cspNonce: opts.context.cspNonce,
        }),
      session: opts.context.session,
    },
    ...(cspNonce ? { ssr: { nonce: cspNonce } } : {}),
    defaultPreload: "intent",
    scrollRestoration: true,
    defaultStructuralSharing: true,
    defaultPreloadStaleTime: 0,
    defaultPendingMinMs: 0,
    dehydrate: () => {
      if (typeof window === "undefined") {
        return { queryClientState: dehydrate(queryClient) };
      }

      return { queryClientState: {} };
    },
    hydrate: (dehydrated: { queryClientState?: unknown }) => {
      if (typeof window !== "undefined" && dehydrated?.queryClientState) {
        hydrate(queryClient, dehydrated.queryClientState);
      }
    },
  });

  return { router, queryClient };
}

export { routeTree };

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof createRouter>["router"];
  }
}
