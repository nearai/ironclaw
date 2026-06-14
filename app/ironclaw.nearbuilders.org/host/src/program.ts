import { serve } from "@hono/node-server";
import { OpenAPIHandler } from "@orpc/openapi/fetch";
import { OpenAPIReferencePlugin } from "@orpc/openapi/plugins";
import { RPCHandler } from "@orpc/server/fetch";
import { BatchHandlerPlugin } from "@orpc/server/plugins";
import { ZodToJsonSchemaConverter } from "@orpc/zod/zod4";
import {
  Cause,
  Effect,
  Exit,
  Fiber,
  FiberHandle,
  Layer,
  ManagedRuntime,
} from "every-plugin/effect";
import { formatORPCError } from "every-plugin/errors";
import { onError } from "every-plugin/orpc";
import { getBaseStyles, getHydrateScript, getThemeInitScript } from "everything-dev/ui/head";
import { type Context, Hono } from "hono";
import { cors } from "hono/cors";
import { proxy } from "hono/proxy";
import { NONCE, secureHeaders } from "hono/secure-headers";
import {
  buildPluginContext,
  createSessionMiddleware,
  type HonoEnv,
  registerAuthHandler,
} from "./services/auth";
import { type ClientRuntimeConfig, ConfigService, type RuntimeConfig } from "./services/config";
import { loadRouterModule, resetFederationInstance } from "./services/federation.server";
import { startIntegrityMonitor } from "./services/integrity-monitor";
import { createPluginsClient, type PluginResult, PluginsService } from "./services/plugins";

import { getTenantRuntimeErrorResponse, resolveRequestRuntime } from "./services/tenant-runtime";
import { logger } from "./utils/logger";

type ActiveRuntimeState = NonNullable<ClientRuntimeConfig["runtime"]>;

type RuntimeClientConfig = ClientRuntimeConfig & { runtime?: ActiveRuntimeState };

type RuntimePlugin = NonNullable<RuntimeConfig["plugins"]>[string];

const BOS_VIEWER_DEFAULT_PATH = "every.near/widget/index";
const BOS_VIEWER_RUNTIME_SCRIPT_URL =
  "https://cdn.jsdelivr.net/npm/near-bos-webcomponent@0.0.9/dist/runtime.25b143da327a5371509f.bundle.js";
const BOS_VIEWER_MAIN_SCRIPT_URL =
  "https://cdn.jsdelivr.net/npm/near-bos-webcomponent@0.0.9/dist/main.1b3f0d7d1017de355a7c.bundle.js";

function normalizeUrl(url?: string | null) {
  if (!url) {
    return null;
  }

  try {
    return new URL(url).toString().replace(/\/$/, "");
  } catch {
    return url.replace(/\/$/, "");
  }
}

function isViewerFramePath(pathname: string) {
  return pathname === "/_viewer" || /^\/_runtime\/[^/]+\/[^/]+\/_viewer\/?$/.test(pathname);
}

function getRuntimeOverride(pathname: string) {
  const match = pathname.match(/^\/_runtime\/([^/]+)\/([^/]+)(?:\/|$)/);
  if (!match) {
    return null;
  }

  const [, encodedAccountId, encodedGatewayId] = match;
  const accountId = decodeURIComponent(encodedAccountId);
  const gatewayId = decodeURIComponent(encodedGatewayId);

  return {
    accountId,
    gatewayId,
    runtimeBasePath: `/_runtime/${encodeURIComponent(accountId)}/${encodeURIComponent(gatewayId)}`,
  };
}

function getFallbackGatewayId(config: RuntimeConfig) {
  if (config.domain) {
    return config.domain;
  }

  return normalizeUrl(config.host?.url)?.replace(/^https?:\/\//, "") ?? "runtime";
}

async function resolveActiveRuntime(config: RuntimeConfig, request: Request) {
  const url = new URL(request.url);
  const override = getRuntimeOverride(url.pathname);

  if (override) {
    return {
      accountId: override.accountId,
      gatewayId: override.gatewayId,
      runtimeBasePath: override.runtimeBasePath,
      title: `${override.accountId}/${override.gatewayId}`,
      description: null,
      hostUrl: url.origin,
    } satisfies ActiveRuntimeState;
  }

  const fallbackGatewayId = getFallbackGatewayId(config);
  return {
    accountId: config.account,
    gatewayId: fallbackGatewayId,
    runtimeBasePath: "/",
    title: config.title ?? config.account,
    description: config.description ?? null,
    hostUrl: url.origin,
  } satisfies ActiveRuntimeState;
}

function buildRuntimeClientConfig(
  config: RuntimeConfig,
  request: Request,
  activeRuntime: ActiveRuntimeState,
  plugins: PluginResult,
): RuntimeClientConfig {
  const requestUrl = new URL(request.url);
  const uiConfig = config.ui;

  if (!uiConfig) {
    throw new Error("UI config is required to build the runtime client config");
  }

  return {
    env: config.env,
    account: activeRuntime.accountId,
    networkId: config.account.endsWith(".testnet") ? "testnet" : "mainnet",
    hostUrl: requestUrl.origin,
    assetsUrl: uiConfig.url,
    apiBase: "/api",
    rpcBase: "/api/rpc",
    authAvailable: plugins.auth !== null,
    repository: config.repository,
    ui: {
      name: uiConfig.name,
      url: uiConfig.url,
      entry: uiConfig.entry,
      integrity: uiConfig.integrity,
    },
    api: config.api
      ? {
          name: config.api.name,
          url: config.api.url,
          entry: config.api.entry,
          integrity: config.api.integrity,
          ...(config.api.variables ? { variables: config.api.variables } : {}),
        }
      : undefined,
    auth: config.auth
      ? {
          name: config.auth.name,
          url: config.auth.url,
          entry: config.auth.entry,
          integrity: config.auth.integrity,
          sidebar: config.auth.sidebar,
          ...(config.auth.variables ? { variables: config.auth.variables } : {}),
        }
      : undefined,
    plugins: Object.fromEntries(
      (Object.entries(config.plugins ?? {}) as Array<[string, RuntimePlugin]>).map(
        ([key, plugin]) => [
          key,
          {
            name: plugin.name,
            url: plugin.url,
            entry: plugin.entry,
            integrity: plugin.integrity,
            ...(plugin.variables ? { variables: plugin.variables } : {}),
            ...(plugin.ui
              ? {
                  ui: {
                    name: plugin.ui.name,
                    url: plugin.ui.url,
                    entry: plugin.ui.entry,
                    source: plugin.ui.source,
                    integrity: plugin.ui.integrity,
                  },
                }
              : {}),
            ...(plugin.sidebar ? { sidebar: plugin.sidebar } : {}),
          },
        ],
      ),
    ),
    runtime: activeRuntime,
  } as RuntimeClientConfig;
}

function extractErrorDetails(error: unknown): {
  message: string;
  stack?: string;
  cause?: string;
} {
  if (!error) return { message: "Unknown error (null/undefined)" };

  if (error instanceof Error) {
    const details: { message: string; stack?: string; cause?: string } = {
      message: error.message || error.name || "Error",
      stack: error.stack,
    };

    if (error.cause) {
      if (error.cause instanceof Error) {
        details.cause = `${error.cause.name}: ${error.cause.message}`;
      } else if (typeof error.cause === "object" && "_tag" in (error.cause as object)) {
        try {
          const squashed = Cause.squash(error.cause as Cause.Cause<unknown>);
          if (squashed instanceof Error) {
            details.cause = `[Effect] ${squashed.name}: ${squashed.message}`;
          } else {
            details.cause = `[Effect] ${String(squashed)}`;
          }
        } catch {
          details.cause = `[Effect Cause] ${JSON.stringify(error.cause)}`;
        }
      } else {
        details.cause = String(error.cause);
      }
    }

    return details;
  }

  if (typeof error === "object" && error !== null) {
    if ("_tag" in error) {
      try {
        const squashed = Cause.squash(error as Cause.Cause<unknown>);
        return extractErrorDetails(squashed);
      } catch {
        return { message: `[Effect] ${JSON.stringify(error)}` };
      }
    }

    if ("message" in error) {
      return { message: String((error as { message: unknown }).message) };
    }

    return { message: JSON.stringify(error) };
  }

  return { message: String(error) };
}

export async function proxyRequest(
  req: Request,
  targetBase: string,
  rewriteCookies = false,
): Promise<Response> {
  const url = new URL(req.url);
  const targetUrl = `${targetBase}${url.pathname}${url.search}`;

  const headers = new Headers(req.headers);
  headers.delete("host");
  headers.set("accept-encoding", "identity");

  if (rewriteCookies) {
    const cookieHeader = headers.get("cookie");
    if (cookieHeader) {
      const rewrittenCookies = cookieHeader.replace(/\bbetter-auth\./g, "__Secure-better-auth.");
      headers.set("cookie", rewrittenCookies);
    }
  }

  const proxyReq = new Request(targetUrl, {
    method: req.method,
    headers,
    body: req.body,
    duplex: "half",
  } as RequestInit);

  const response = await fetch(proxyReq);

  const responseHeaders = new Headers(response.headers);
  responseHeaders.delete("content-encoding");
  responseHeaders.delete("content-length");

  if (rewriteCookies) {
    responseHeaders.delete("set-cookie");
    const setCookies =
      typeof response.headers.getSetCookie === "function"
        ? response.headers.getSetCookie()
        : (response.headers.get("set-cookie")?.split(/,(?=\s*(?:__Secure-|__Host-)?\w+=)/) ?? []);
    for (const cookie of setCookies) {
      const rewritten = cookie
        .replace(/^(__Secure-|__Host-)/i, "")
        .replace(/;\s*Domain=[^;]*/gi, "")
        .replace(/;\s*Secure/gi, "");
      responseHeaders.append("set-cookie", rewritten);
    }
  }

  return new Response(response.body, {
    status: response.status,
    statusText: response.statusText,
    headers: responseHeaders,
  });
}

function buildStaticAssetProxyHeaders(req: Request) {
  const headers = new Headers();

  for (const name of ["accept", "accept-language", "if-none-match", "if-modified-since"]) {
    const value = req.headers.get(name);
    if (value) {
      headers.set(name, value);
    }
  }

  return headers;
}

async function proxyStaticAssetRequest(req: Request, targetBase: string): Promise<Response> {
  const url = new URL(req.url);
  const targetUrl = `${targetBase}${url.pathname}${url.search}`;

  return proxy(targetUrl, {
    raw: req,
    headers: buildStaticAssetProxyHeaders(req),
  });
}

export function setupApiRoutes(
  app: Hono<HonoEnv>,
  config: RuntimeConfig,
  plugins: PluginResult,
  sessionMiddleware: ReturnType<typeof createSessionMiddleware>,
  loadingState: {
    status: string;
    startTime: number;
    milestones: string[];
    error: Error | null;
    ssrEnabled: boolean;
  },
) {
  const apiConfig = config.api;

  if (!apiConfig) {
    throw new Error("API config is required to start the host");
  }

  const getHealthStatus = () => {
    const elapsed = Date.now() - loadingState.startTime;
    return {
      status: loadingState.status,
      ssr: loadingState.ssrEnabled
        ? loadingState.status === "ready"
          ? "available"
          : "unavailable"
        : "disabled",
      auth: plugins.auth
        ? { mounted: true, name: plugins.auth.name }
        : { mounted: false, name: null },
      plugins: {
        loaded: plugins.status.loadedPlugins,
        ...(plugins.status.error ? { error: plugins.status.error } : {}),
      },
      uptime: elapsed,
      milestones: loadingState.milestones,
      ...(loadingState.error ? { error: loadingState.error.message } : {}),
    };
  };

  const isProxyMode = !!apiConfig.proxy;

  const publicRpcRouters = new Map<string, RPCHandler<any>>();

  const registerPublicRpcRouter = (prefix: string, router: unknown) => {
    publicRpcRouters.set(
      prefix,
      new RPCHandler(router as any, {
        plugins: [new BatchHandlerPlugin()],
        interceptors: [
          onError((error: unknown) => {
            formatORPCError(error);
            throw error;
          }),
        ],
      }),
    );
  };

  if (plugins.auth?.router) {
    registerPublicRpcRouter("/api/rpc/auth", plugins.auth.router);
  }

  for (const [pluginKey, plugin] of Object.entries(plugins.plugins)) {
    if (pluginKey === "ironclaw") continue;
    registerPublicRpcRouter(`/api/rpc/${pluginKey}`, plugin.router);
  }

  const getPublicRpcRoute = (pathname: string) => {
    for (const [prefix, handler] of publicRpcRouters.entries()) {
      if (pathname === prefix || pathname.startsWith(`${prefix}/`)) {
        return { prefix, handler };
      }
    }

    return null;
  };

  if (isProxyMode) {
    const proxyTarget = apiConfig.proxy!;
    logger.info(`[API] Proxy mode enabled → ${proxyTarget}`);

    app.all("/api/*", async (c: Context<HonoEnv>) => {
      if (c.req.path === "/api/_health") {
        return c.json(getHealthStatus());
      }
      const response = await proxyRequest(c.req.raw, proxyTarget, true);
      return response;
    });

    return;
  }

  app.get("/api/_health", (c: Context<HonoEnv>) => {
    return c.json(getHealthStatus());
  });

  app.use("/api/*", sessionMiddleware);

  const handleOrpc = async (
    c: Context<HonoEnv>,
    handler: RPCHandler<any> | OpenAPIHandler<any>,
    prefix: `/${string}`,
  ) => {
    const context = buildPluginContext(c);

    const result = await handler.handle(c.req.raw, { prefix, context });
    return result.response
      ? c.newResponse(result.response.body, result.response)
      : c.text("Not Found", 404);
  };

  const apiRouter = plugins.api?.router;

  if (!apiRouter) {
    const unavailable = (c: Context<HonoEnv>) =>
      c.json({ error: "Service Unavailable", message: "The API is currently unavailable." }, 503);

    app.all("/api/rpc", unavailable);
    app.all("/api/rpc/*", unavailable);
    app.all("/api", unavailable);
    app.all("/api/*", unavailable);
    return;
  }

  const rpcHandler = new RPCHandler(apiRouter as any, {
    plugins: [new BatchHandlerPlugin()],
    interceptors: [
      onError((error: unknown) => {
        formatORPCError(error);
        throw error;
      }),
    ],
  });

  const apiHandler = new OpenAPIHandler(apiRouter as any, {
    plugins: [
      new OpenAPIReferencePlugin({
        schemaConverters: [new ZodToJsonSchemaConverter()],
        specGenerateOptions: {
          info: {
            title: `${config.title ?? config.account} API`,
            version: "1.0.0",
          },
          servers: [{ url: "/api" }, { url: `${config.host?.url ?? ""}/api` }],
        },
      }),
    ],
    interceptors: [
      onError((error: unknown) => {
        formatORPCError(error);
        throw error;
      }),
    ],
  });

  app.all("/api/rpc", (c: Context<HonoEnv>) => handleOrpc(c, rpcHandler, "/api/rpc"));
  app.all("/api/rpc/*", (c: Context<HonoEnv>) => {
    const publicRoute = getPublicRpcRoute(c.req.path);
    if (publicRoute) {
      return handleOrpc(c, publicRoute.handler, publicRoute.prefix as `/${string}`);
    }

    return handleOrpc(c, rpcHandler, "/api/rpc");
  });
  app.all("/api", (c: Context<HonoEnv>) => handleOrpc(c, apiHandler, "/api"));
  app.all("/api/*", (c: Context<HonoEnv>) => handleOrpc(c, apiHandler, "/api"));
}

export const createStartServer = (onReady?: () => void) =>
  Effect.gen(function* () {
    const port = Number(process.env.PORT) || 3000;
    const isDev = process.env.NODE_ENV !== "production";

    if (!process.env.CORS_ORIGIN && !isDev) {
      logger.warn(
        "[Security] CORS_ORIGIN is not set in production. Auth endpoints will reject cross-origin requests.",
      );
      logger.warn(
        "[Security] Set CORS_ORIGIN to your allowed origins (comma-separated), e.g.: CORS_ORIGIN=https://yourdomain.com,https://app.yourdomain.com",
      );
    }

    const config = yield* ConfigService;
    const uiConfig = config.ui!;
    const plugins = yield* PluginsService;

    const app = new Hono<HonoEnv>();

    app.onError((err: unknown, c: Context<HonoEnv>) => {
      const details = extractErrorDetails(err);
      logger.error(`[Hono Error] ${c.req.method} ${c.req.path}`);
      logger.error(`[Hono Error] Message: ${details.message}`);
      if (details.cause) {
        logger.error(`[Hono Error] Cause: ${details.cause}`);
      }
      if (details.stack) {
        logger.error(`[Hono Error] Stack:\n${details.stack}`);
      }
      return c.json({ error: details.message, cause: details.cause }, 500);
    });

    const allowedOrigins = process.env.CORS_ORIGIN?.split(",").map((o: string) => o.trim()) ?? [
      config.host?.url ?? "",
      ...(uiConfig.url ? [uiConfig.url] : []),
    ];

    app.use(
      "/*",
      cors({
        origin: (origin) => {
          if (!origin) return "*";
          if (allowedOrigins.includes(origin)) return origin;
          if (origin.startsWith("https://")) return origin;
          if (isDev && origin.startsWith("http://")) return origin;
          return null;
        },
        credentials: true,
      }),
    );

    const remoteOrigins = [
      ...(uiConfig.url ? [new URL(uiConfig.url).origin] : []),
      ...(config.api?.url ? [new URL(config.api.url).origin] : []),
      ...(config.auth?.url ? [new URL(config.auth.url).origin] : []),
      ...Object.values(config.plugins ?? {}).flatMap((p: RuntimePlugin) => {
        const origins: string[] = [];
        if (p.url) origins.push(new URL(p.url).origin);
        if (p.ui?.url) origins.push(new URL(p.ui.url).origin);
        return origins;
      }),
    ];

    const uniqueOrigins = [...new Set(remoteOrigins)];

    const wsOrigins = isDev
      ? uniqueOrigins.filter((o) => o.startsWith("http:")).map((o) => o.replace(/^http:/, "ws:"))
      : [];

    const CSP_STRICT = process.env.CSP_STRICT === "false" ? false : !isDev;

    const cdnOrigins = ["https://cdn.jsdelivr.net", "https://unpkg.com"];

    const cspScriptSrc = CSP_STRICT
      ? [NONCE, "'strict-dynamic'", "'unsafe-eval'"]
      : ["'self'", "'unsafe-inline'", "'unsafe-eval'", "https:", ...uniqueOrigins, ...cdnOrigins];

    app.use("*", (c, next) => {
      const frameAncestors = isViewerFramePath(c.req.path) ? ["'self'"] : ["'none'"];

      const viewerPath = isViewerFramePath(c.req.path);

      return secureHeaders({
        crossOriginOpenerPolicy: "same-origin-allow-popups",
        contentSecurityPolicy: {
          defaultSrc: ["'self'"],
          scriptSrc: cspScriptSrc,
          styleSrc: ["'self'", "'unsafe-inline'", "https:", ...uniqueOrigins, ...cdnOrigins],
          imgSrc: [
            "'self'",
            "data:",
            ...(isDev ? ["http:"] : ["https:"]),
            ...(uiConfig.url ? [new URL(uiConfig.url).origin] : []),
          ],
          connectSrc: ["'self'", "https:", "http:", ...uniqueOrigins, ...wsOrigins, ...cdnOrigins],
          fontSrc: viewerPath
            ? ["'self'", "data:", "https:", ...uniqueOrigins]
            : ["'self'", "https:", ...uniqueOrigins],
          manifestSrc: [
            "'self'",
            "https:",
            ...(uiConfig.url ? [new URL(uiConfig.url).origin] : []),
          ],
          frameSrc: ["'self'", "https:", ...uniqueOrigins],
          objectSrc: ["'none'"],
          baseUri: ["'self'"],
          formAction: ["'self'"],
          frameAncestors,
          workerSrc: ["'self'", "https:", ...uniqueOrigins],
        },
      })(c, next);
    });

    app.get("/health", (c: Context<HonoEnv>) => c.text("OK"));

    const loadingState = {
      status: "ready" as "loading" | "ready" | "failed",
      startTime: Date.now(),
      milestones: [] as string[],
      error: null as Error | null,
      ssrEnabled: Boolean(uiConfig.ssrUrl),
    };

    const renderClientShell = (
      ctx: Context<HonoEnv>,
      runtimeSourceConfig: RuntimeConfig,
      runtimeConfig: ClientRuntimeConfig,
      error?: Error | null,
    ) => {
      const nonce = CSP_STRICT ? ctx.get("secureHeadersNonce") : undefined;
      const uiIntegrity = runtimeSourceConfig.ui.integrity;
      const assetsUrl = runtimeConfig.assetsUrl.replace(/\/$/, "");
      const themeInitScript = (getThemeInitScript() as { children?: string }).children ?? "";
      const hydrateScript =
        (
          getHydrateScript(
            runtimeConfig as Partial<ClientRuntimeConfig>,
            undefined,
            undefined,
            nonce,
          ) as {
            children?: string;
          }
        ).children ?? "";

      const uiVersion = uiIntegrity ? `?v=${encodeURIComponent(uiIntegrity)}` : "";
      const sriAttr = uiIntegrity ? ` integrity="${uiIntegrity}" crossorigin="anonymous"` : "";
      const nonceAttr = nonce ? ` nonce="${nonce}"` : "";

      const pluginUiScripts = Object.entries(runtimeSourceConfig.plugins ?? {})
        .filter(([, p]: [string, RuntimePlugin]) => p.ui?.url && p.ui.source === "remote")
        .map(([pluginKey, p]: [string, RuntimePlugin]) => {
          const uiSri = p.ui!.integrity
            ? ` integrity="${p.ui!.integrity}" crossorigin="anonymous"`
            : "";
          const pluginVersion = p.ui!.integrity ? `?v=${encodeURIComponent(p.ui!.integrity)}` : "";
          return `<script${nonceAttr} src="/__mf/plugin-ui/${pluginKey}/remoteEntry.js${pluginVersion}"${uiSri}></script>`;
        })
        .join("\n");

      return ctx.html(
        `<!DOCTYPE html>
          <html lang="en">
            <head>
              <meta charset="utf-8" />
              <meta name="viewport" content="width=device-width, initial-scale=1.0, viewport-fit=cover" />
              <title>${runtimeConfig.runtime?.title ?? runtimeSourceConfig.title ?? runtimeSourceConfig.account}</title>
              <link rel="icon" type="image/x-icon" href="/favicon.ico" />
              <link rel="icon" type="image/svg+xml" href="/icon.svg" />
              <link rel="manifest" href="/manifest.json" />
              <link rel="stylesheet" href="${assetsUrl}/static/css/style.css${uiVersion}" />
              <style>
                ${getBaseStyles()}
                .shell { min-height: 100vh; min-height: 100dvh; display: flex; align-items: center; justify-content: center; }
                .fade { animation: fadeIn 0.3s ease-in; }
                @keyframes fadeIn { from { opacity: 0; } to { opacity: 1; } }
                .error { color: #fca5a5; }
              </style>
              <script${nonceAttr} src="${assetsUrl}/remoteEntry.js${uiVersion}"${sriAttr}></script>
              ${pluginUiScripts}
              <script${nonceAttr}>${themeInitScript}</script>
              <script${nonceAttr}>${hydrateScript}</script>
            </head>
            <body>
              <div id="root">
                <div class="shell">
                  <div class="fade">
                    ${
                      error
                        ? `<p class="error">SSR unavailable, showing client app.</p><p>${error.message}</p>`
                        : `<p>Loading...</p>`
                    }
                  </div>
                </div>
              </div>
            </body>
          </html>`,
        200,
      );
    };

    const proxyUiAssetRequest = async (c: Context<HonoEnv>) => {
      const runtime = await resolveRequestRuntime(config, c.req.raw, {
        verification: "stale-while-revalidate",
      });
      return await proxyStaticAssetRequest(c.req.raw, runtime.config.ui.url);
    };

    const sessionMiddleware = createSessionMiddleware(plugins);

    registerAuthHandler(app, plugins);
    setupApiRoutes(app, config, plugins, sessionMiddleware, loadingState);

    app.on(["GET", "HEAD"], "*", async (c: Context<HonoEnv>, next) => {
      const { pathname } = new URL(c.req.url);

      if (
        pathname === "/" ||
        pathname.startsWith("/api/") ||
        pathname.startsWith("/__mf/") ||
        pathname.startsWith("/_runtime/") ||
        pathname === "/health"
      ) {
        return next();
      }

      const lastSegment = pathname.split("/").pop() ?? "";
      if (
        !/\.(js|css|png|jpg|jpeg|gif|svg|ico|json|woff2?|ttf|eot|webp|avif|map|txt|xml)$/i.test(
          lastSegment,
        )
      ) {
        return next();
      }

      try {
        return await proxyUiAssetRequest(c);
      } catch (error) {
        const { message, status } = getTenantRuntimeErrorResponse(error);
        return c.text(message, { status: status as 404 | 500 | 502 });
      }
    });

    for (const [pluginKey, pluginConfig] of Object.entries(config.plugins ?? {}) as Array<
      [string, RuntimePlugin]
    >) {
      if (!pluginConfig.ui?.url) continue;
      const proxyPrefix = `/__mf/plugin-ui/${pluginKey}`;
      app.all(`${proxyPrefix}/*`, async (c: Context<HonoEnv>) => {
        try {
          const runtime = await resolveRequestRuntime(config, c.req.raw, {
            verification: "stale-while-revalidate",
          });
          const pluginUiUrl = runtime.config.plugins?.[pluginKey]?.ui?.url;
          if (!pluginUiUrl) {
            return c.text(`Plugin UI unavailable for ${pluginKey}`, 404);
          }
          return await proxyStaticAssetRequest(c.req.raw, pluginUiUrl);
        } catch (error) {
          const { message, status } = getTenantRuntimeErrorResponse(error);
          return c.text(message, { status: status as 404 | 500 | 502 });
        }
      });
    }

    const renderBosViewer = (c: Context<HonoEnv>) => {
      const nonce = CSP_STRICT ? c.get("secureHeadersNonce") : undefined;
      const nonceAttr = nonce ? ` nonce="${nonce}"` : "";
      const widgetPath =
        new URL(c.req.url).searchParams.get("path")?.trim() || BOS_VIEWER_DEFAULT_PATH;
      const widgetPathJson = JSON.stringify(widgetPath);

      c.header("X-Robots-Tag", "noindex, nofollow");

      return c.html(
        `<!DOCTYPE html>
          <html lang="en">
            <head>
              <meta charset="utf-8" />
              <meta name="viewport" content="width=device-width, initial-scale=1.0, viewport-fit=cover" />
              <meta name="robots" content="noindex, nofollow" />
              <title>Viewer</title>
              <style>
                html, body, #viewer-root { height: 100%; margin: 0; }
                body { background: #fff; overflow: hidden; }
                near-social-viewer { display: block; width: 100%; height: 100%; }
              </style>
              <script${nonceAttr}>
                (function() {
                  var widgetPath = ${widgetPathJson};
                  while (widgetPath.startsWith("/")) {
                    widgetPath = widgetPath.slice(1);
                  }
                  history.replaceState(null, "", "/" + widgetPath);
                })();
              </script>
              <script${nonceAttr} src="${BOS_VIEWER_RUNTIME_SCRIPT_URL}"></script>
              <script${nonceAttr} src="${BOS_VIEWER_MAIN_SCRIPT_URL}"></script>
            </head>
            <body>
              <div id="viewer-root"></div>
              <script${nonceAttr}>
                (function() {
                  var widgetPath = ${widgetPathJson};
                  while (widgetPath.startsWith("/")) {
                    widgetPath = widgetPath.slice(1);
                  }
                  var mount = function() {
                    var root = document.getElementById("viewer-root");
                    if (!root || root.querySelector("near-social-viewer")) return;
                    var viewer = document.createElement("near-social-viewer");
                    viewer.setAttribute("src", widgetPath);
                    viewer.setAttribute("network", "mainnet");
                    root.appendChild(viewer);
                  };

                  if (customElements.get("near-social-viewer")) {
                    mount();
                    return;
                  }

                  customElements.whenDefined("near-social-viewer").then(mount);
                })();
              </script>
            </body>
          </html>`,
      );
    };

    app.get("/_viewer", renderBosViewer);
    app.get("/_runtime/:accountId/:gatewayId/_viewer", renderBosViewer);

    app.use("/*", sessionMiddleware);

    app.get("*", async (c: Context<HonoEnv>) => {
      let resolvedRuntime: Awaited<ReturnType<typeof resolveRequestRuntime>>;
      try {
        resolvedRuntime = await resolveRequestRuntime(config, c.req.raw, {
          verification: "blocking",
        });
      } catch (error) {
        const { message, status } = getTenantRuntimeErrorResponse(error);
        return c.text(message, { status: status as 404 | 500 | 502 });
      }

      const effectiveConfig = resolvedRuntime.config;
      const activeRuntime = await resolveActiveRuntime(effectiveConfig, c.req.raw);
      const nonce = CSP_STRICT ? c.get("secureHeadersNonce") : undefined;
      const runtimeConfig = buildRuntimeClientConfig(
        effectiveConfig,
        c.req.raw,
        activeRuntime,
        plugins,
      );

      if (!effectiveConfig.ui.ssrUrl) {
        return renderClientShell(c, effectiveConfig, runtimeConfig);
      }

      const routerModuleResult = await Effect.runPromise(
        loadRouterModule(effectiveConfig).pipe(Effect.either),
      );

      if (routerModuleResult._tag === "Left") {
        logger.error("[SSR] Failed to load Router module:", routerModuleResult.left);
        return renderClientShell(c, effectiveConfig, runtimeConfig, routerModuleResult.left);
      }

      const ssrRouterModule = routerModuleResult.right;

      try {
        const pluginContext = buildPluginContext(c);
        const ssrApiClient = createPluginsClient(plugins, pluginContext);

        const render = () =>
          ssrRouterModule?.renderToStream(c.req.raw, {
            session: c.get("session") ? { session: c.get("session"), user: c.get("user") } : null,
            basepath: runtimeConfig.runtime?.runtimeBasePath,
            runtimeConfig,
            apiClient: ssrApiClient,
            cspNonce: nonce,
          });

        const result = await render();
        const responseHeaders = new Headers(result?.headers);
        const cspHeader = c.res.headers.get("Content-Security-Policy");
        if (cspHeader) {
          responseHeaders.set("Content-Security-Policy", cspHeader);
        }
        return new Response(result?.stream, {
          status: result?.statusCode,
          headers: responseHeaders,
        });
      } catch (error) {
        logger.error("[SSR] Streaming error:", error);
        return c.html(
          `
        <!DOCTYPE html>
        <html lang="en">
          <head>
            <meta charset="utf-8" />
            <title>Server Error</title>
            <style>
              body { font-family: system-ui; padding: 2rem; background: #1c1c1e; color: #fafafa; }
              pre { background: #2d2d2d; padding: 1rem; border-radius: 8px; overflow-x: auto; }
            </style>
          </head>
          <body>
            <h1>Server Error</h1>
            <p>An error occurred during server-side rendering.</p>
            <pre>${error instanceof Error ? error.stack : String(error)}</pre>
          </body>
        </html>
      `,
          500,
        );
      }
    });

    const startHttpServer = () => {
      const hostname = process.env.HOST || "0.0.0.0";

      const proxiedFetch = (req: Request): Response | Promise<Response> => {
        const url = new URL(req.url);
        const forwardedProto = req.headers.get("x-forwarded-proto");
        const forwardedHost = req.headers.get("x-forwarded-host");

        if (forwardedProto) {
          url.protocol = forwardedProto;
        }
        if (forwardedHost) {
          url.host = forwardedHost;
        }

        if (forwardedProto || forwardedHost) {
          req = new Request(url, req);
        }

        return app.fetch(req);
      };

      const server = serve({ fetch: proxiedFetch, port, hostname }, () => {
        logger.info(
          `[Server] Host ${isDev ? "dev" : "production"} server running at http://${hostname}:${port}`,
        );
        onReady?.();
      });
      return server;
    };

    const httpServer = startHttpServer();

    yield* Effect.addFinalizer(() =>
      Effect.async<void, never>((resume) => {
        logger.info("[Server] Closing HTTP server...");
        httpServer.close(() => {
          logger.info("[Server] HTTP server closed");
          resume(Effect.void);
        });
      }),
    );

    yield* Effect.never;
  });

export interface ServerInput {
  config: RuntimeConfig;
}

export interface ServerHandle {
  ready: Promise<void>;
  shutdown: () => Promise<void>;
}

export const runServer = (input: ServerInput): ServerHandle => {
  const ConfigLive = Layer.succeed(ConfigService, input.config);
  const ServerLive = Layer.provideMerge(PluginsService.Live, ConfigLive);

  const stopMonitor = startIntegrityMonitor(input.config);

  const runtime = ManagedRuntime.make(ServerLive);
  let programFiber: Fiber.RuntimeFiber<void, unknown> | null = null;

  const ready = new Promise<void>((resolveReady, rejectReady) => {
    const serverEffect = createStartServer(() => resolveReady());

    const program = Effect.gen(function* () {
      const handle = yield* FiberHandle.make();
      yield* FiberHandle.run(handle, serverEffect);
      yield* FiberHandle.join(handle);
    }).pipe(Effect.scoped);

    programFiber = runtime.runFork(program);

    programFiber.addObserver((exit) => {
      if (Exit.isFailure(exit) && !Cause.isInterruptedOnly(exit.cause)) {
        rejectReady(Cause.squash(exit.cause));
      }
    });
  });

  const shutdown = async () => {
    logger.info("[Server] Shutting down...");
    stopMonitor();

    if (programFiber) {
      await Effect.runPromise(
        Fiber.interrupt(programFiber).pipe(
          Effect.timeout("5 seconds"),
          Effect.catchAll(() => Effect.void),
        ),
      );
    }

    await runtime.dispose();
    resetFederationInstance();
    logger.info("[Server] Shutdown complete");
  };

  return { ready, shutdown };
};

export const runServerBlocking = async (input: ServerInput) => {
  const handle = runServer(input);

  const forceExit = () => {
    console.log("\n[Server] Force exit");
    process.exit(0);
  };

  const gracefulShutdown = () => {
    const timeout = setTimeout(forceExit, 5000);
    handle
      .shutdown()
      .then(() => {
        clearTimeout(timeout);
        process.exit(0);
      })
      .catch(() => {
        clearTimeout(timeout);
        process.exit(1);
      });
  };

  process.on("SIGINT", gracefulShutdown);
  process.on("SIGTERM", gracefulShutdown);

  try {
    await handle.ready;
    await new Promise(() => {});
  } catch (err) {
    console.error("Failed to start server:", err);
    process.exit(1);
  }
};
