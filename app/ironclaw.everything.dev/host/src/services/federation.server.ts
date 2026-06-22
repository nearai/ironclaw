import { createInstance } from "@module-federation/enhanced/runtime";
import { Effect, Schedule } from "every-plugin/effect";
import { verifySriForUrl } from "everything-dev/integrity";
import type { RouterModule } from "../types";
import type { RuntimeConfig } from "./config";
import { FederationError } from "./errors";

export type { RouterModule };

const ROUTER_MODULE_CACHE_TTL_MS = 5 * 60_000;
const SSR_INTEGRITY_CACHE_TTL_MS = 5 * 60_000;
const MAX_ROUTER_MODULE_CACHE_SIZE = 128;
const MAX_SSR_INTEGRITY_CACHE_SIZE = 256;

interface CachedPromise<T> {
  expiresAt: number;
  value: Promise<T>;
}

const routerModuleCache = new Map<string, CachedPromise<RouterModule>>();
const verifiedSsrEntryCache = new Map<string, CachedPromise<void>>();

function pruneExpiredCacheEntries<T>(cache: Map<string, CachedPromise<T>>, now: number) {
  for (const [key, entry] of cache.entries()) {
    if (entry.expiresAt <= now) {
      cache.delete(key);
    }
  }
}

function enforceCacheLimit<T>(cache: Map<string, CachedPromise<T>>, maxSize: number) {
  while (cache.size > maxSize) {
    const oldestKey = cache.keys().next().value;
    if (!oldestKey) break;
    cache.delete(oldestKey);
  }
}

export function resetFederationInstance() {
  routerModuleCache.clear();
  verifiedSsrEntryCache.clear();
}

function shouldCacheRouterModule(config: RuntimeConfig) {
  return config.ui.source !== "local" && Boolean(config.ui.ssrIntegrity);
}

async function verifySsrEntryIntegrity(entryUrl: string, expectedIntegrity: string): Promise<void> {
  const cacheKey = `${entryUrl}::${expectedIntegrity}`;
  const now = Date.now();
  pruneExpiredCacheEntries(verifiedSsrEntryCache, now);

  const cached = verifiedSsrEntryCache.get(cacheKey);
  if (cached && cached.expiresAt > now) {
    return cached.value;
  }

  const verification = verifySriForUrl(entryUrl, expectedIntegrity, {
    resolveEntryUrl: false,
  }).catch((error) => {
    verifiedSsrEntryCache.delete(cacheKey);
    throw error;
  });

  verifiedSsrEntryCache.set(cacheKey, {
    value: verification,
    expiresAt: now + SSR_INTEGRITY_CACHE_TTL_MS,
  });
  enforceCacheLimit(verifiedSsrEntryCache, MAX_SSR_INTEGRITY_CACHE_SIZE);
  return verification;
}

function getSsrEntryUrl(config: RuntimeConfig) {
  const isLocalDev = config.ui.source === "local";
  const ssrUrl = config.ui.ssrUrl ?? (isLocalDev ? config.ui.url : undefined);

  if (!ssrUrl) {
    if (!isLocalDev) {
      throw new FederationError({
        remoteName: config.ui.name,
        cause: new Error(
          "SSR URL not configured in production. Set app.ui.ssr in bos.config.json to enable SSR.",
        ),
      });
    }
    throw new Error(
      "SSR URL not configured. In local dev, set app.ui.ssr or use a UI package with SSR support.",
    );
  }

  const entryUrl = `${ssrUrl.replace(/\/$/, "")}/remoteEntry.server.js`;
  if (!isLocalDev && config.ui.ssrIntegrity) {
    return `${entryUrl}?v=${encodeURIComponent(config.ui.ssrIntegrity)}`;
  }

  return entryUrl;
}

const retrySchedule = Schedule.addDelay(Schedule.recurs(5), () => 500);

export const loadRouterModule = (config: RuntimeConfig) =>
  Effect.gen(function* () {
    const useCache = shouldCacheRouterModule(config);
    const ssrEntryUrl = getSsrEntryUrl(config);

    if (config.ui.ssrIntegrity) {
      yield* Effect.tryPromise({
        try: () => verifySsrEntryIntegrity(ssrEntryUrl, config.ui.ssrIntegrity!),
        catch: (e) =>
          new FederationError({
            remoteName: config.ui.name,
            remoteUrl: config.ui.ssrUrl,
            cause: e instanceof Error ? e : new Error(String(e)),
          }),
      });
    }

    const cacheKey = `${config.ui.name}::${ssrEntryUrl}::${config.ui.ssrIntegrity ?? "no-integrity"}`;
    const now = Date.now();
    if (useCache) {
      pruneExpiredCacheEntries(routerModuleCache, now);
    }
    let cached = useCache ? routerModuleCache.get(cacheKey) : undefined;

    if (!cached || cached.expiresAt <= now) {
      const value = Effect.runPromise(
        Effect.retry(
          Effect.gen(function* () {
            const mf = createInstance({
              name: `host-${Buffer.from(cacheKey).toString("base64url")}`,
              remotes: [
                {
                  name: config.ui.name,
                  entry: ssrEntryUrl,
                  alias: config.ui.name,
                },
              ],
            });

            return yield* Effect.tryPromise({
              try: async () => {
                const result = await mf.loadRemote<any>(`${config.ui.name}/Router`, {
                  from: "build",
                });

                if (!result) {
                  throw new Error(`Module not found: ${config.ui.name}/Router`);
                }

                return result.default as RouterModule;
              },
              catch: (e) =>
                new FederationError({
                  remoteName: config.ui.name,
                  remoteUrl: config.ui.ssrUrl,
                  cause: e,
                }),
            });
          }),
          retrySchedule,
        ),
      ).catch((error) => {
        if (useCache) {
          routerModuleCache.delete(cacheKey);
        }
        throw error;
      });
      if (useCache) {
        cached = { value, expiresAt: now + ROUTER_MODULE_CACHE_TTL_MS };
        routerModuleCache.set(cacheKey, cached);
        enforceCacheLimit(routerModuleCache, MAX_ROUTER_MODULE_CACHE_SIZE);
      } else {
        cached = { value, expiresAt: now };
      }
    }

    const loadedModule = yield* Effect.tryPromise({
      try: () => cached.value,
      catch: (e) =>
        new FederationError({
          remoteName: config.ui.name,
          remoteUrl: config.ui.ssrUrl,
          cause: e,
        }),
    });

    return loadedModule;
  }).pipe(
    Effect.timeout("30 seconds"),
    Effect.tapError((error: Error) => Effect.logError(`[SSR] Failed: ${error.message}`)),
  );
