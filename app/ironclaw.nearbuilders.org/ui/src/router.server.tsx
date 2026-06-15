import { dehydrate, hydrate, QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createMemoryHistory, createRouter as createTanStackRouter } from "@tanstack/react-router";
import {
  createRequestHandler,
  RouterServer,
  renderRouterToStream,
} from "@tanstack/react-router/ssr/server";
import { collectHeadData } from "everything-dev/ui/router";
import type {
  CreateRouterOptions,
  HeadData,
  RenderOptions,
  RenderResult,
  RouterContext,
} from "./app";
import { createApiClient, createAuthClient } from "./app";
import { routeTree } from "./routeTree.gen";

export type { CreateRouterOptions, HeadData, RenderOptions, RenderResult, RouterContext };

function defaultErrorComponent({ error }: { error: Error }) {
  return (
    <div className="min-h-screen flex items-center justify-center bg-background">
      <div className="text-center">
        <h1 className="text-4xl font-bold text-foreground mb-4">Oops!</h1>
        <p className="text-muted-foreground mb-4">Something went wrong</p>
        <details className="text-sm text-muted-foreground bg-muted p-4 rounded mb-8">
          <summary className="cursor-pointer">Error Details</summary>
          <pre className="mt-2 whitespace-pre-wrap text-left">{error.message}</pre>
        </details>
      </div>
    </div>
  );
}

function defaultNotFoundComponent() {
  return (
    <div className="min-h-screen flex items-center justify-center bg-background px-6">
      <div className="text-center">
        <h1 className="text-3xl font-semibold text-foreground">Not Found</h1>
        <p className="mt-2 text-muted-foreground">The requested page could not be found.</p>
      </div>
    </div>
  );
}

function defaultPendingComponent() {
  return (
    <div className="min-h-screen flex items-center justify-center bg-background px-6">
      <p className="text-sm text-muted-foreground">Loading...</p>
    </div>
  );
}

const createRouter = (opts: CreateRouterOptions) => {
  const queryClient = opts.context.queryClient;

  const history = opts.history ?? createMemoryHistory();

  const cspNonce = opts.context.cspNonce;
  const router = createTanStackRouter({
    routeTree,
    history,
    basepath: opts.basepath,
    context: {
      queryClient,
      runtimeConfig: opts.context.runtimeConfig,
      apiClient: opts.context.apiClient,
      authClient:
        opts.context.authClient ??
        createAuthClient({
          runtimeConfig: opts.context.runtimeConfig,
          cspNonce: opts.context.cspNonce,
        }),
      session: opts.context.session,
      cspNonce,
    },
    ...(cspNonce ? { ssr: { nonce: cspNonce } } : {}),
    defaultPreload: "intent",
    scrollRestoration: true,
    defaultStructuralSharing: true,
    defaultPreloadStaleTime: 0,
    defaultErrorComponent,
    defaultNotFoundComponent,
    defaultPendingComponent,
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
};

const getRouteHead = async (pathname: string, context?: Partial<RouterContext>) => {
  const history = createMemoryHistory({ initialEntries: [pathname] });
  const queryClient = new QueryClient();
  const runtimeConfig = context?.runtimeConfig;
  if (!runtimeConfig?.hostUrl || !runtimeConfig.rpcBase) {
    throw new Error("Missing runtime config for route head generation");
  }

  const router = createTanStackRouter({
    routeTree,
    history,
    context: {
      queryClient,
      runtimeConfig,
      apiClient:
        context?.apiClient ??
        createApiClient({ hostUrl: runtimeConfig.hostUrl, rpcBase: runtimeConfig.rpcBase }),
      authClient:
        context?.authClient ?? createAuthClient({ runtimeConfig, cspNonce: context?.cspNonce }),
      session: context?.session,
    },
  });

  return collectHeadData(router);
};

const renderToStream = async (request: Request, renderOptions: RenderOptions) => {
  const url = new URL(request.url);
  const history = createMemoryHistory({ initialEntries: [url.pathname + url.search] });
  let queryClientRef: QueryClient | null = null;

  const handler = createRequestHandler({
    request,
    createRouter: () => {
      const localQueryClient =
        queryClientRef ??
        new QueryClient({
          defaultOptions: {
            queries: {
              staleTime: 5 * 60 * 1000,
              gcTime: 30 * 60 * 1000,
              refetchOnWindowFocus: false,
              retry: 1,
            },
          },
        });
      const { router } = createRouter({
        history,
        basepath: renderOptions.basepath,
        context: {
          queryClient: localQueryClient,
          runtimeConfig: renderOptions.runtimeConfig,
          apiClient: renderOptions.apiClient,
          authClient: createAuthClient({
            runtimeConfig: renderOptions.runtimeConfig,
            headers: request.headers,
            cspNonce: renderOptions.cspNonce,
          }),
          session: renderOptions.session,
          cspNonce: renderOptions.cspNonce,
        },
      });
      queryClientRef = localQueryClient;
      return router;
    },
  });

  const response = await handler(({ request, responseHeaders, router }) =>
    renderRouterToStream({
      request,
      responseHeaders,
      router,
      children: (
        <QueryClientProvider client={queryClientRef!}>
          <RouterServer router={router} />
        </QueryClientProvider>
      ),
    }),
  );

  return {
    stream: response.body!,
    statusCode: response.status,
    headers: response.headers,
  } satisfies RenderResult;
};

const routerModule = {
  createRouter,
  getRouteHead,
  renderToStream,
  routeTree,
} as const;

export default routerModule;
