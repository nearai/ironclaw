import { createApiClient, createAuthClient, getCspNonce, getRuntimeConfig } from "./app";
import "./styles.css";

declare global {
  interface Window {
    __EVERYTHING_DEV_HYDRATE_PROMISE__?: Promise<void>;
    $_TSR?: unknown;
  }
}

export async function hydrate() {
  if (window.__EVERYTHING_DEV_HYDRATE_PROMISE__) {
    return window.__EVERYTHING_DEV_HYDRATE_PROMISE__;
  }

  window.__EVERYTHING_DEV_HYDRATE_PROMISE__ = (async () => {
    console.log("[Hydrate] Starting...");

    const runtimeConfig = getRuntimeConfig();
    const cspNonce = getCspNonce();

    const { QueryClientProvider } = await import("@tanstack/react-query");
    const { createRouter } = await import("./router");
    const client = new (await import("@tanstack/react-query")).QueryClient({
      defaultOptions: {
        queries: {
          staleTime: 5 * 60 * 1000,
          gcTime: 30 * 60 * 1000,
          refetchOnWindowFocus: false,
          retry: 1,
        },
      },
    });

    if (!runtimeConfig.hostUrl || !runtimeConfig.rpcBase) {
      throw new Error("Missing hostUrl or rpcBase in runtime config");
    }

    const { router } = createRouter({
      context: {
        queryClient: client,
        runtimeConfig,
        cspNonce,
        apiClient: createApiClient({
          hostUrl: runtimeConfig.hostUrl,
          rpcBase: runtimeConfig.rpcBase,
        }),
        authClient: createAuthClient({ runtimeConfig, cspNonce }),
      },
    });

    if (window.$_TSR) {
      const { hydrateRoot } = await import("react-dom/client");
      const { RouterClient } = await import("@tanstack/react-router/ssr/client");

      console.log("[Hydrate] Calling hydrateRoot...");
      hydrateRoot(
        document,
        <QueryClientProvider client={client}>
          <RouterClient router={router} />
        </QueryClientProvider>,
      );
    } else {
      const { createRoot } = await import("react-dom/client");
      const { RouterProvider } = await import("@tanstack/react-router");

      console.log("[Hydrate] Calling createRoot...");
      createRoot(document).render(
        <QueryClientProvider client={client}>
          <RouterProvider router={router} />
        </QueryClientProvider>,
      );
    }

    console.log("[Hydrate] Complete!");
  })();

  return window.__EVERYTHING_DEV_HYDRATE_PROMISE__;
}

export default hydrate;
