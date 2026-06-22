import { loadResolvedConfig } from "everything-dev/config";
import type { ClientRuntimeConfig } from "everything-dev/types";
import type { RenderOptionsWithApi, RouterContext } from "everything-dev/ui/types";
import type { RuntimeConfig } from "@/types";
import type { ApiClient } from "../../../ui/src/lib/api";
import type { AuthClient } from "../../../ui/src/lib/auth";

export async function loadTestRuntimeConfig(): Promise<RuntimeConfig> {
  const result = await loadResolvedConfig();

  if (!result) {
    throw new Error("No bos.config.json found for host tests");
  }

  const config = result.runtime;
  const rawUi = result.config.app.ui;

  if (!config.ui.url && rawUi.production) {
    config.ui.url = rawUi.production.replace(/\/$/, "");
  }
  if (!config.ui.ssrUrl && rawUi.ssr) {
    config.ui.ssrUrl = rawUi.ssr.replace(/\/$/, "");
  }

  return config;
}

export function createMockAuthClient(): AuthClient {
  const noOp = (): Promise<{ data: null; error: null }> =>
    Promise.resolve({ data: null, error: null });
  const handler: ProxyHandler<() => Promise<{ data: null; error: null }>> = {
    get(_target, prop) {
      if (prop === "$Infer") return { Session: null, Account: null };
      if (prop === "getHeaders") return () => ({});
      if (typeof prop === "symbol") return undefined;
      return new Proxy(noOp, handler);
    },
  };
  return new Proxy(noOp, handler) as unknown as AuthClient;
}

export function buildTestClientRuntimeConfig(config: RuntimeConfig): Partial<ClientRuntimeConfig> {
  const plugins: NonNullable<Partial<ClientRuntimeConfig>["plugins"]> = {};

  for (const [key, plugin] of Object.entries(config.plugins ?? {}) as Array<
    [
      string,
      {
        name: string;
        url: string;
        entry: string;
        variables?: Record<string, import("everything-dev/types").JsonValue>;
      },
    ]
  >) {
    plugins[key] = {
      name: plugin.name,
      url: plugin.url,
      entry: plugin.entry,
      ...(plugin.variables ? { variables: plugin.variables } : {}),
    };
  }

  return {
    env: config.env,
    account: config.account,
    networkId: config.networkId,
    hostUrl: config.host?.url,
    assetsUrl: config.ui.url,
    apiBase: "/api",
    rpcBase: "/api/rpc",
    repository: config.repository,
    runtime: {
      accountId: config.account,
      gatewayId: config.domain ?? config.account,
      runtimeBasePath: "/",
      title: config.title ?? config.account,
      description: config.description ?? null,
      hostUrl: config.host?.url,
    },
    ui: {
      name: config.ui.name,
      url: config.ui.url,
      entry: config.ui.entry,
    },
    api: {
      name: config.api.name,
      url: config.api.url,
      entry: config.api.entry,
      ...(config.api.variables ? { variables: config.api.variables } : {}),
    },
    auth: config.auth
      ? {
          name: config.auth.name,
          url: config.auth.url,
          entry: config.auth.entry,
          ...(config.auth.sidebar ? { sidebar: config.auth.sidebar } : {}),
          ...(config.auth.variables ? { variables: config.auth.variables } : {}),
        }
      : undefined,
    plugins: Object.keys(plugins).length > 0 ? plugins : undefined,
  };
}

export function buildTestRouteHeadContext(config: RuntimeConfig): Partial<RouterContext> {
  return {
    runtimeConfig: buildTestClientRuntimeConfig(config),
  };
}

export function buildTestRenderOptions(
  config: RuntimeConfig,
  apiClient: ApiClient,
  authClient?: AuthClient,
): RenderOptionsWithApi<ApiClient> {
  return {
    runtimeConfig: buildTestClientRuntimeConfig(config),
    apiClient,
    session: null,
    authClient,
  } satisfies RenderOptionsWithApi<ApiClient>;
}
