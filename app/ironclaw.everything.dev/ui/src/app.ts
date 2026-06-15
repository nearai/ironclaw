/**
 * Public UI surface — runtime helpers, client factories, and router types.
 *
 * ⚠️  DO NOT MODIFY THIS FILE.
 * It is framework-owned and will be overwritten by `bos sync` / `bos upgrade`.
 *
 * Imports within this file must be relative paths (./lib/api, ./lib/auth).
 * Never import from "@/app" here — that would create a circular self-reference.
 *
 * This file re-exports everything that UI route code needs and defines
 * thin runtime helpers (getAccount, getAppName, etc.) derived from
 * the injected runtime config.
 *
 * Framework file roles (understand this boundary — don't dig into host):
 *
 *   hydrate.tsx       — Client bootstrap. Creates browser-side QueryClient,
 *                        Router, and browser-side auth/API clients once.
 *                        Called from the host-rendered HTML shell.
 *
 *   router.tsx        — Client router factory. Consumes the context set up
 *                        during hydration. Uses browser history.
 *
 *   router.server.tsx — SSR router factory. Creates request-scoped server
 *                        router and server-side API/auth clients per request.
 *                        Mirrors client router shape for hydration consistency.
 *
 *   routes/__root.tsx — HTML shell, head/scripts/styles, runtime config
 *                        handoff. Root boundary between host-rendered
 *                        document and the UI application.
 *
 *   app.ts            — This file. Re-exports the minimal shared runtime
 *                        helpers plus public client surfaces from ./lib/api
 *                        and ./lib/auth. Also re-exports router-facing
 *                        public types.
 *
 *   Boundary rule: The host loads UI remotely via Module Federation and
 * provides runtime config + auth/API routing. Work within the typed
 * surface exported here. Only investigate host internals if something
 * is genuinely broken and a host PR is warranted.
 */

export { getBaseStyles } from "everything-dev/ui/head";

import {
  buildPublishedAccountHref,
  buildPublishedGatewayHref,
  buildRuntimeHref,
  getCspNonce,
  getRuntimeConfig,
} from "everything-dev/ui/runtime";

export {
  buildPublishedAccountHref,
  buildPublishedGatewayHref,
  buildRuntimeHref,
  getCspNonce,
  getRuntimeConfig,
};

type RuntimeConfigInput = Partial<import("everything-dev/types").ClientRuntimeConfig> | undefined;

function readRuntimeConfig(config?: RuntimeConfigInput) {
  if (config) return config;
  if (typeof window === "undefined") return undefined;
  try {
    return getRuntimeConfig();
  } catch {
    return undefined;
  }
}

export function getActiveRuntime(config?: RuntimeConfigInput) {
  return readRuntimeConfig(config)?.runtime;
}

export function getAccount(config?: RuntimeConfigInput): string {
  return readRuntimeConfig(config)?.account ?? "every.near";
}

export function getRepository(config?: RuntimeConfigInput): string | undefined {
  return readRuntimeConfig(config)?.repository;
}

export function getAppName(config?: RuntimeConfigInput): string {
  return getActiveRuntime(config)?.title ?? getAccount(config);
}

import type { ApiClient } from "./lib/api";
import type { AuthClient as AuthClientType } from "./lib/auth";

export type { ApiClient } from "./lib/api";
export { createApiClient, useApiClient, useOrpc } from "./lib/api";
export type { AuthClient, Organization, Passkey, SessionData } from "./lib/auth";
export { createAuthClient, sessionQueryOptions, useAuthClient, useRelayHistory } from "./lib/auth";

import type {
  CreateRouterOptions as BaseCreateRouterOptions,
  RenderOptions as BaseRenderOptions,
  RouterContextWithApi as BaseRouterContextWithApi,
} from "everything-dev/ui/types";
import type { SessionData } from "./lib/auth";

export type {
  ClientRuntimeConfig,
  ClientRuntimeInfo,
} from "everything-dev/types";
export type {
  HeadData,
  HeadLink,
  HeadMeta,
  HeadScript,
  RenderResult,
  RouterModule,
} from "everything-dev/ui/types";

export interface RouterContext extends BaseRouterContextWithApi<ApiClient, SessionData> {
  apiClient: ApiClient;
  authClient: AuthClientType;
}

export interface CreateRouterOptions
  extends Omit<BaseCreateRouterOptions<ApiClient, SessionData>, "context"> {
  context: RouterContext;
}

export interface RenderOptions extends Omit<BaseRenderOptions<SessionData>, "runtimeConfig"> {
  runtimeConfig: BaseRenderOptions<SessionData>["runtimeConfig"];
  apiClient: ApiClient;
  authClient?: AuthClientType;
}
