import { createORPCClient, onError } from "@orpc/client";
import { RPCLink } from "@orpc/client/fetch";
import type { ContractRouterClient } from "@orpc/contract";
import { createTanstackQueryUtils } from "@orpc/tanstack-query";
import { useRouter } from "@tanstack/react-router";
import type { ApiContract } from "./api-types.gen";

export type { ApiContract };
export type ApiClient = ContractRouterClient<ApiContract>;

let browserApiClient: ApiClient | null = null;

function createRpcLink(runtimeConfig: { hostUrl: string; rpcBase: string }) {
  return new RPCLink({
    url: `${runtimeConfig.hostUrl}${runtimeConfig.rpcBase}`,
    interceptors: [
      onError((error: unknown) => {
        console.error("oRPC API Error:", error);

        if (typeof window === "undefined") {
          return;
        }

        if (error && typeof error === "object" && "message" in error) {
          const message = String(error.message).toLowerCase();
          if (
            message.includes("fetch") ||
            message.includes("network") ||
            message.includes("failed to fetch")
          ) {
            void import("sonner").then(({ toast }) => {
              toast.error("Unable to connect to API", {
                id: "api-connection-error",
                description: "The API is currently unavailable. Please try again later.",
              });
            });
          }
        }
      }),
    ],
    fetch(url: RequestInfo | URL, options?: RequestInit) {
      return fetch(url, {
        ...options,
        credentials: "include",
      });
    },
  });
}

export function createApiClient(runtimeConfig: { hostUrl: string; rpcBase: string }): ApiClient {
  if (!runtimeConfig.hostUrl) {
    throw new Error("Missing runtime host URL");
  }

  if (typeof window !== "undefined" && browserApiClient) {
    return browserApiClient;
  }

  const client: ApiClient = createORPCClient(
    createRpcLink({
      hostUrl: runtimeConfig.hostUrl,
      rpcBase: runtimeConfig.rpcBase,
    }),
  );

  if (typeof window !== "undefined") {
    browserApiClient = client;
  }

  return client;
}

export function useApiClient(): ApiClient {
  return useRouter().options.context.apiClient;
}

export function useOrpc() {
  const client = useApiClient();
  return createTanstackQueryUtils(client);
}
