import { createInstance, getInstance } from "@module-federation/enhanced/runtime";
import { setGlobalFederationInstance } from "@module-federation/runtime-core";
import { createPluginRuntime } from "every-plugin";
import { Context, Effect, Layer } from "every-plugin/effect";
import { IntegrityRegistry, verifyConfigAgainstChain } from "everything-dev/integrity";
import { installIntegrityFetchHook } from "everything-dev/mf";
import type { RuntimeConfig, SharedConfig } from "everything-dev/types";
import { ConfigService } from "./config";
import { PluginError } from "./errors";

export interface InitializedPluginResult {
  context: unknown;
  [key: string]: unknown;
}

export interface HostPluginEntry {
  key: string;
  name: string;
  createClient: (context?: unknown) => unknown;
  router: unknown;
  metadata: { remoteUrl: string; version?: string };
  initialized?: InitializedPluginResult;
}

export interface PluginStatus {
  available: boolean;
  pluginName: string | null;
  error: string | null;
  errorDetails: string | null;
  loadedPlugins: string[];
}

export interface PluginResult {
  runtime: ReturnType<typeof createPluginRuntime> | null;
  auth: HostPluginEntry | null;
  api: HostPluginEntry | null;
  plugins: Record<string, HostPluginEntry>;
  authClient: ((ctx?: unknown) => unknown) | null;
  status: PluginStatus;
}

function secretsFromEnv(keys: string[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (const k of keys) {
    const v = process.env[k];
    if (typeof v === "string" && v.length > 0) out[k] = v;
  }
  return out;
}

function formatError(error: unknown): string {
  if (error instanceof Error) {
    let msg = error.message || error.name || "Error";
    if (error.cause instanceof Error) {
      msg += ` (caused by: ${error.cause.message || error.cause.name})`;
    } else if (error.cause) {
      msg += ` (caused by: ${String(error.cause)})`;
    }
    return msg;
  }
  if (typeof error === "object" && error !== null) {
    const err = error as Record<string, unknown>;
    if (err.message) return String(err.message);
    if (err._tag) return `[${err._tag}] ${JSON.stringify(error)}`;
    return JSON.stringify(error);
  }
  return String(error);
}

function normalizeDomain(domain: string | undefined, env: string): string | undefined {
  if (!domain) return domain;
  if (/^https?:\/\//.test(domain)) return domain;
  if (env === "development" && /^(localhost|127\.0\.0\.1)/.test(domain)) {
    return `http://${domain}`;
  }
  return `https://${domain}`;
}

function mergeSharedMaps(
  ...maps: Array<Record<string, SharedConfig> | undefined>
): Record<string, SharedConfig> {
  const merged: Record<string, SharedConfig> = {};
  for (const map of maps) {
    if (!map) continue;
    for (const [name, config] of Object.entries(map)) {
      const existing = merged[name];
      if (existing && !isSameSharedConfig(existing, config)) {
        throw new Error(`Conflicting shared dependency "${name}" in runtime config`);
      }
      merged[name] = config;
    }
  }
  return merged;
}

function normalizeSharedConfig(config: SharedConfig): Record<string, unknown> {
  return {
    version: config.version,
    requiredVersion: config.requiredVersion ?? false,
    singleton: config.singleton ?? false,
    strictVersion: config.strictVersion ?? false,
    eager: config.eager ?? false,
    shareScope: config.shareScope ?? "default",
  };
}

function isSameSharedConfig(a: SharedConfig, b: SharedConfig): boolean {
  const left = normalizeSharedConfig(a);
  const right = normalizeSharedConfig(b);
  return (
    left.version === right.version &&
    left.requiredVersion === right.requiredVersion &&
    left.singleton === right.singleton &&
    left.strictVersion === right.strictVersion &&
    left.eager === right.eager &&
    left.shareScope === right.shareScope
  );
}

function collectPluginSharedDeps(config: RuntimeConfig): Record<string, SharedConfig> {
  const shared: Record<string, SharedConfig> = {};
  for (const plugin of Object.values(config.plugins ?? {})) {
    if (plugin.shared && Object.keys(plugin.shared).length > 0) {
      for (const [name, sharedConfig] of Object.entries(plugin.shared)) {
        const existing = shared[name];
        if (existing && !isSameSharedConfig(existing, sharedConfig)) {
          throw new Error(`Conflicting shared dependency "${name}" across plugins at runtime`);
        }
        shared[name] = sharedConfig;
      }
    }
  }
  return shared;
}

/**
 * Pre-registers app-specific shared dependencies in the Module Federation runtime.
 * This runs in the host scope before every-plugin initializes its own core-only MF instance.
 */
async function registerAppSharedDeps(
  appShared: Record<string, SharedConfig> | undefined,
): Promise<void> {
  if (!appShared || Object.keys(appShared).length === 0) return;

  const sharedEntries: Record<
    string,
    {
      version: string;
      shareScope: string;
      get: () => Promise<() => unknown>;
      shareConfig: {
        singleton: boolean;
        requiredVersion: string | false;
        strictVersion: boolean;
        eager: boolean;
      };
    }
  > = {};

  for (const [name, config] of Object.entries(appShared)) {
    try {
      // Import from host scope — this is where app-specific deps are installed
      const mod = await import(name);
      sharedEntries[name] = {
        version: config.version,
        shareScope: config.shareScope ?? "default",
        get: () => Promise.resolve(() => mod),
        shareConfig: {
          singleton: config.singleton ?? false,
          requiredVersion: config.requiredVersion ?? false,
          strictVersion: config.strictVersion ?? false,
          eager: config.eager ?? false,
        },
      };
    } catch (error) {
      console.error(`[Plugins] Failed to preload shared dependency ${name}: ${formatError(error)}`);
      throw new Error(
        `Shared dependency "${name}" is configured in bos.config.json but could not be resolved. ` +
          `Ensure it is installed in the host workspace.`,
      );
    }
  }

  let instance = getInstance();
  if (!instance) {
    instance = createInstance({
      name: "host",
      remotes: [],
      shared: sharedEntries,
    });
    setGlobalFederationInstance(instance);
    console.log(
      `[Plugins] Pre-registered ${Object.keys(sharedEntries).length} app-specific shared dep(s)`,
    );
  } else {
    instance.registerShared(sharedEntries);
    console.log(
      `[Plugins] Augmented existing MF instance with ${Object.keys(sharedEntries).length} app-specific shared dep(s)`,
    );
  }
}

const unavailableResult = (
  pluginName: string | null,
  error: string | null,
  errorDetails: string | null,
  loadedPlugins: string[] = [],
): PluginResult => ({
  runtime: null,
  auth: null,
  api: null,
  plugins: {},
  authClient: null,
  status: { available: false, pluginName, error, errorDetails, loadedPlugins },
});

type RuntimePluginInput = NonNullable<RuntimeConfig["plugins"]>[string];

interface RuntimePluginEntry {
  key: string;
  runtimeId: string;
  config: RuntimeConfig["api"] | RuntimePluginInput;
}

function buildRegistryEntries(config: RuntimeConfig): RuntimePluginEntry[] {
  const entries: RuntimePluginEntry[] = [];
  if (config.api?.url) {
    entries.push({ key: "api", runtimeId: config.api.name, config: config.api });
  }
  for (const [key, plugin] of Object.entries(config.plugins ?? {})) {
    if (plugin.url) {
      entries.push({ key, runtimeId: plugin.name, config: plugin });
    }
  }
  return entries;
}

function collectSecrets(config: { secrets?: string[] }): Record<string, string> {
  return secretsFromEnv(config.secrets ?? []);
}

async function loadPluginEntry(
  runtime: any,
  entry: RuntimePluginEntry,
  integrityRegistry: IntegrityRegistry,
  pluginsClient?: Record<string, unknown>,
  baseVariables?: Record<string, unknown>,
): Promise<HostPluginEntry> {
  if (entry.config.integrity) {
    integrityRegistry.registerEntry(entry.config.url, entry.config.integrity);
  }

  const variables: Record<string, unknown> = { ...baseVariables, ...entry.config.variables };
  const args: [unknown, unknown?] = [{ variables, secrets: collectSecrets(entry.config) }];
  if (pluginsClient) args.push(pluginsClient);

  const result = await runtime.usePlugin(entry.runtimeId, ...args);

  return { key: entry.key, name: entry.config.name, ...result };
}

export const initializePlugins = Effect.gen(function* () {
  const config: RuntimeConfig = yield* ConfigService;

  if (config.api.proxy) {
    console.log(`[Plugins] Proxy mode enabled, skipping plugin initialization`);
    console.log(`[Plugins] API requests will be proxied to: ${config.api.proxy}`);
    return {
      runtime: null,
      auth: null,
      api: null,
      plugins: {},
      authClient: null,
      status: {
        available: false,
        pluginName: config.api.name,
        error: null,
        errorDetails: null,
        loadedPlugins: [],
      },
    } satisfies PluginResult;
  }

  const registryEntries = buildRegistryEntries(config);
  if (registryEntries.length === 0 && !config.auth) {
    console.log("[Plugins] No remote plugins configured, using host API only");
    return unavailableResult(config.api.name, null, null);
  }

  console.log(`[Plugins] Registering ${registryEntries.length} plugin(s)`);

  if (config.env === "production" && config.account) {
    const bosUrl = `bos://${config.account}/${config.domain ?? "everything.dev"}`;
    verifyConfigAgainstChain(config as unknown as Record<string, unknown>, bosUrl)
      .then(({ verified, mismatches }) => {
        if (!verified) {
          console.error(
            `[Attestation] Config integrity does not match on-chain anchor. Mismatches: ${mismatches.join(", ")}`,
          );
        }
      })
      .catch(() => {});
  }

  const result = yield* Effect.tryPromise({
    try: async () => {
      const allEntries: RuntimePluginEntry[] = [];

      if (config.auth?.url) {
        allEntries.push({ key: "auth", runtimeId: config.auth.name, config: config.auth });
      }

      allEntries.push(...registryEntries);

      const integrityRegistry = new IntegrityRegistry();

      const allEntriesWithUrls = allEntries.filter((e) => e.config.url);
      console.log(
        `[Plugins] Registry entries: ${allEntriesWithUrls.map((e) => `${e.key}=${e.config.url}`).join(", ") || "none"}`,
      );

      // Pre-register app-specific shared deps in host scope before every-plugin initializes
      await registerAppSharedDeps(
        mergeSharedMaps(config.api.shared, config.auth?.shared, collectPluginSharedDeps(config)),
      );

      const runtime = createPluginRuntime({
        registry: Object.fromEntries(
          allEntries.map((entry) => [entry.runtimeId, { remote: entry.config.url }]),
        ),
        secrets: {},
      });

      const mfInstance = (runtime as any).__mfInstance as any | undefined;
      if (mfInstance) {
        installIntegrityFetchHook(mfInstance, integrityRegistry);
      }

      // Phase 0: Load auth plugin (app-level infrastructure)
      let authPlugin: HostPluginEntry | null = null;
      let authClient: ((ctx?: unknown) => unknown) | null = null;
      if (config.auth?.url) {
        const authEntry: RuntimePluginEntry = {
          key: "auth",
          runtimeId: config.auth.name,
          config: config.auth,
        };
        try {
          const normalizedDomain = normalizeDomain(
            config.env === "development" ? "localhost:3000" : config.domain,
            config.env,
          );
          const authBaseVariables: Record<string, unknown> = {
            account: config.account,
            domain: normalizedDomain,
            hostUrl: normalizedDomain,
          };

          const corsOrigin = process.env.CORS_ORIGIN?.split(",").map((o) => o.trim());
          if (corsOrigin) {
            authBaseVariables.trustedOrigins = corsOrigin;
          }

          authPlugin = await loadPluginEntry(
            runtime,
            authEntry,
            integrityRegistry,
            undefined,
            authBaseVariables,
          );
          authClient = authPlugin.createClient;
          console.log(`[Plugins] Auth plugin loaded: ${authPlugin.name}`);
        } catch (error) {
          console.error(`[Plugins] Failed to load auth plugin: ${formatError(error)}`);
        }
      }

      // Phase 1: Load all non-API plugins
      const pluginEntries = registryEntries.filter((e) => e.key !== "api");

      const pluginResults = await Promise.allSettled(
        pluginEntries.map((entry) => loadPluginEntry(runtime, entry, integrityRegistry)),
      );

      const loadedPlugins: Record<string, HostPluginEntry> = {};
      const loadedPluginKeys: string[] = [];
      const pluginsClient: Record<string, unknown> = {};
      const errors: string[] = [];

      pluginResults.forEach((result, index) => {
        const entry = pluginEntries[index];
        const key = entry?.key ?? "unknown";
        if (result.status === "fulfilled") {
          loadedPlugins[key] = result.value;
          loadedPluginKeys.push(key);
          pluginsClient[key] = result.value.createClient;
        } else {
          const msg = formatError(result.reason);
          errors.push(msg);
          pluginsClient[key] = () => {
            throw new Error(`Plugin "${key}" failed to load: ${msg}`);
          };
        }
      });

      // Phase 2: Load the API plugin with pluginsClient + authClient
      let baseApi: HostPluginEntry | null = null;
      const apiEntry = registryEntries.find((e) => e.key === "api");

      if (apiEntry) {
        try {
          const apiPluginsClient: Record<string, unknown> = { ...pluginsClient };
          if (authClient) {
            apiPluginsClient.auth = authClient;
          }

          baseApi = await loadPluginEntry(runtime, apiEntry, integrityRegistry, apiPluginsClient);
          loadedPlugins.api = baseApi;
          loadedPluginKeys.unshift("api");
        } catch (error) {
          errors.push(formatError(error));
        }
      }

      return {
        runtime,
        auth: authPlugin,
        api: baseApi,
        plugins: loadedPlugins,
        authClient,
        status: {
          available: Boolean(baseApi),
          pluginName: config.api.name,
          error: errors.length > 0 ? errors.join("; ") : null,
          errorDetails: errors.length > 0 ? errors.join("\n") : null,
          loadedPlugins: loadedPluginKeys,
        },
      } satisfies PluginResult;
    },
    catch: (error) =>
      new PluginError({
        pluginName: config.api.name,
        pluginUrl: config.api.url,
        cause: error,
      }),
  });

  return result;
}).pipe(
  Effect.catchAll((error) => {
    const pluginName = error instanceof PluginError ? error.pluginName : null;
    const pluginUrl = error instanceof PluginError ? error.pluginUrl : null;
    const errorMessage = formatError(error);
    const errorStack = error instanceof Error ? error.stack : undefined;

    console.error("[Plugins] ❌ Failed to initialize plugin");
    console.error(`[Plugins] Plugin: ${pluginName}`);
    console.error(`[Plugins] URL: ${pluginUrl}`);
    console.error(`[Plugins] Error: ${errorMessage}`);
    console.warn("[Plugins] Server will continue without plugin functionality");

    return Effect.succeed(unavailableResult(pluginName ?? null, errorMessage, errorStack ?? null));
  }),
);

export class PluginsService extends Context.Tag("host/PluginsService")<
  PluginsService,
  PluginResult
>() {
  static Live = Layer.scoped(
    PluginsService,
    Effect.gen(function* () {
      const plugins = yield* initializePlugins;

      yield* Effect.addFinalizer(() =>
        Effect.promise(async () => {
          if (plugins.runtime) {
            console.log("[Plugins] Shutting down plugin runtime...");
            await plugins.runtime.shutdown();
          }
        }),
      );

      return plugins;
    }),
  );
}

export function createPluginsClient(result: PluginResult, context?: unknown): unknown {
  const apiClient = result.api?.createClient(context);

  // Do NOT Object.assign the result — apiClient is a Proxy and assign would copy
  // only static own-properties, silently dropping Proxy-resolved RPC methods.

  const pluginClients: Record<string, unknown> = {};
  for (const [key, plugin] of Object.entries(result.plugins)) {
    if (key === "api") continue;
    pluginClients[key] = plugin.createClient(context);
  }

  if (result.authClient) {
    pluginClients.auth = result.authClient(context);
  }

  if (!apiClient) {
    return pluginClients;
  }

  return new Proxy(apiClient, {
    get(target, key) {
      if (typeof key === "string" && key in pluginClients) {
        return pluginClients[key];
      }
      return Reflect.get(target, key);
    },
    has(target, key) {
      if (key in pluginClients) return true;
      return Reflect.has(target, key);
    },
  });
}
