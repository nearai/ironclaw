import {
  buildRuntimeConfig,
  isRuntimeOverrideAllowed,
  loadRemoteConfig,
  parseRuntimeOverrideTargets,
  type RuntimeConfig,
} from "everything-dev/config";
import { verifySriForUrl } from "everything-dev/integrity";
import { logger } from "../utils/logger";

const REMOTE_CONFIG_TTL_MS = 30_000;
const VERIFICATION_TTL_MS = 5 * 60_000;
const MAX_REMOTE_CONFIG_CACHE_SIZE = 256;
const MAX_VERIFICATION_CACHE_SIZE = 512;
const NEAR_ACCOUNT_ID_REGEX =
  /^(?=.{2,64}$)([a-z0-9]+(?:[-_][a-z0-9]+)*)(\.([a-z0-9]+(?:[-_][a-z0-9]+)*))*$/;

type RuntimeOverrideTarget = ReturnType<typeof parseRuntimeOverrideTargets>[number];
type BosEnv = "development" | "production" | "staging";
type RuntimePlugin = NonNullable<RuntimeConfig["plugins"]>[string];
type IntegrityVerificationMode = "blocking" | "stale-while-revalidate";

interface ResolveRequestRuntimeOptions {
  verification?: IntegrityVerificationMode;
}

interface CachedRemoteConfig {
  expiresAt: number;
  value: Promise<Awaited<ReturnType<typeof loadRemoteConfig>>>;
}

interface CachedVerification {
  expiresAt: number;
  value: Promise<void>;
  refreshing?: Promise<void>;
}

export interface RequestRuntimeResolution {
  config: RuntimeConfig;
  tenantAccountId: string | null;
  gatewayId: string;
  ssrAllowed: boolean;
}

export class TenantRuntimeError extends Error {
  status: number;

  constructor(message: string, status: number, options?: { cause?: unknown }) {
    super(message, options);
    this.name = "TenantRuntimeError";
    this.status = status;
  }
}

const remoteConfigCache = new Map<string, CachedRemoteConfig>();
const verifiedUiCache = new Map<string, CachedVerification>();
const unsupportedOverrideWarnings = new Set<string>();
let tenantWhitelistCache: { raw: string; value: Set<string> } | null = null;
let allowedOverridesCache: { raw: string; value: RuntimeOverrideTarget[] } | null = null;

function pruneExpiredCacheEntries<T extends { expiresAt: number }>(
  cache: Map<string, T>,
  now: number,
) {
  for (const [key, entry] of cache.entries()) {
    if (entry.expiresAt <= now) {
      cache.delete(key);
    }
  }
}

function enforceCacheLimit<T>(cache: Map<string, T>, maxSize: number) {
  while (cache.size > maxSize) {
    const oldestKey = cache.keys().next().value;
    if (!oldestKey) break;
    cache.delete(oldestKey);
  }
}

function getTenantRuntimeErrorStatus(error: unknown): number {
  if (error instanceof TenantRuntimeError) {
    return error.status;
  }

  return 500;
}

export function getTenantRuntimeErrorResponse(error: unknown): { status: number; message: string } {
  return {
    status: getTenantRuntimeErrorStatus(error),
    message: error instanceof Error ? error.message : String(error),
  };
}

export function clearTenantRuntimeCaches() {
  remoteConfigCache.clear();
  verifiedUiCache.clear();
  unsupportedOverrideWarnings.clear();
  tenantWhitelistCache = null;
  allowedOverridesCache = null;
}

function normalizeDomain(domain: string | undefined, fallbackHostUrl: string): string {
  if (domain && domain.length > 0) {
    return domain;
  }

  try {
    return new URL(fallbackHostUrl).hostname;
  } catch {
    return (
      fallbackHostUrl
        .replace(/^https?:\/\//, "")
        .replace(/\/$/, "")
        .split(":")[0] ?? ""
    );
  }
}

function parseBoolean(value: string | undefined): boolean {
  if (!value) return false;
  return ["1", "true", "yes", "on"].includes(value.toLowerCase());
}

function getTenantWhitelist(): Set<string> {
  const raw = process.env.TENANT_WHITELIST ?? "";
  if (tenantWhitelistCache?.raw === raw) {
    return tenantWhitelistCache.value;
  }

  const value = new Set(
    raw
      .split(",")
      .map((entry) => entry.trim())
      .filter(Boolean),
  );
  tenantWhitelistCache = { raw, value };
  return value;
}

function getAllowedOverrides(): RuntimeOverrideTarget[] {
  const raw = process.env.ALLOW_OVERRIDE ?? "";
  if (allowedOverridesCache?.raw === raw) {
    return allowedOverridesCache.value;
  }

  const value = parseRuntimeOverrideTargets(raw);
  allowedOverridesCache = { raw, value };
  return value;
}

function warnUnsupportedOverrideTargets(targets: ReadonlyArray<RuntimeOverrideTarget>) {
  for (const target of targets) {
    if (target === "ui" || target === "plugins" || target.startsWith("plugins.")) {
      continue;
    }

    if (!unsupportedOverrideWarnings.has(target)) {
      unsupportedOverrideWarnings.add(target);
      logger.warn(
        `[Tenant Runtime] Ignoring unsupported override target "${target}" in fixed-core mode`,
      );
    }
  }
}

function resolveTenantAccountId(
  hostname: string,
  gatewayId: string,
  namespaceAccountId: string,
): string | null {
  const normalizedHost = hostname.toLowerCase();
  const normalizedGateway = gatewayId.toLowerCase();
  const normalizedNamespaceAccountId = namespaceAccountId.toLowerCase();

  if (
    normalizedHost === normalizedGateway ||
    normalizedHost === "localhost" ||
    normalizedHost === "127.0.0.1"
  ) {
    return null;
  }

  const suffix = `.${normalizedGateway}`;
  if (!normalizedHost.endsWith(suffix)) {
    return null;
  }

  const tenantLabel = normalizedHost.slice(0, -suffix.length);
  const tenantSegments = tenantLabel.split(".").filter(Boolean);
  if (tenantSegments.length === 0 || tenantSegments.join(".") !== tenantLabel) {
    throw new TenantRuntimeError(`Invalid tenant host: ${hostname}`, 404);
  }

  const accountId = `${tenantSegments.join(".")}.${normalizedNamespaceAccountId}`;
  if (!NEAR_ACCOUNT_ID_REGEX.test(accountId)) {
    throw new TenantRuntimeError(`Invalid tenant account: ${accountId}`, 404);
  }

  return accountId;
}

function getRemoteConfigCached(bosUrl: string, env: BosEnv) {
  const now = Date.now();
  pruneExpiredCacheEntries(remoteConfigCache, now);
  const cached = remoteConfigCache.get(bosUrl);

  if (cached && cached.expiresAt > now) {
    return cached.value;
  }

  const value = loadRemoteConfig(bosUrl, env).catch((error) => {
    remoteConfigCache.delete(bosUrl);
    const message = error instanceof Error ? error.message : String(error);
    if (message.includes(`No config found for ${bosUrl}`)) {
      throw new TenantRuntimeError(`No tenant config found for ${bosUrl}`, 404, { cause: error });
    }
    throw new TenantRuntimeError(`Failed to load tenant config for ${bosUrl}`, 502, {
      cause: error,
    });
  });
  remoteConfigCache.set(bosUrl, { value, expiresAt: now + REMOTE_CONFIG_TTL_MS });
  enforceCacheLimit(remoteConfigCache, MAX_REMOTE_CONFIG_CACHE_SIZE);
  return value;
}

function createIntegrityFailure(label: string, error: unknown) {
  return new TenantRuntimeError(`Integrity check failed for ${label}`, 502, { cause: error });
}

function createVerificationPromise(
  cacheKey: string,
  url: string,
  integrity: string,
  label: string,
) {
  const verification = verifySriForUrl(url, integrity).catch((error) => {
    const cached = verifiedUiCache.get(cacheKey);
    if (cached?.value === verification || cached?.refreshing === verification) {
      verifiedUiCache.delete(cacheKey);
    }
    throw createIntegrityFailure(label, error);
  });

  return verification;
}

function scheduleVerificationRefresh(
  cacheKey: string,
  cached: CachedVerification,
  url: string,
  integrity: string,
  label: string,
) {
  if (cached.refreshing) {
    return cached.refreshing;
  }

  const refresh = createVerificationPromise(cacheKey, url, integrity, label)
    .then(() => {
      const entry = verifiedUiCache.get(cacheKey);
      if (!entry || entry.refreshing !== refresh) {
        return;
      }

      entry.value = Promise.resolve();
      entry.expiresAt = Date.now() + VERIFICATION_TTL_MS;
      entry.refreshing = undefined;
    })
    .catch((error) => {
      logger.error(
        `[Tenant Runtime] Integrity refresh failed for ${label}: ${error instanceof Error ? error.message : String(error)}`,
      );
      throw error;
    });

  cached.refreshing = refresh;
  return refresh;
}

async function verifyIntegrity(
  url: string,
  integrity: string,
  label: string,
  mode: IntegrityVerificationMode,
) {
  const cacheKey = `${url}::${integrity}`;
  const now = Date.now();
  const cached = verifiedUiCache.get(cacheKey);

  if (cached && cached.expiresAt > now) {
    return cached.value;
  }

  if (!cached) {
    const value = createVerificationPromise(cacheKey, url, integrity, label);
    verifiedUiCache.set(cacheKey, {
      value,
      expiresAt: now + VERIFICATION_TTL_MS,
    });
    enforceCacheLimit(verifiedUiCache, MAX_VERIFICATION_CACHE_SIZE);
    await value;

    const entry = verifiedUiCache.get(cacheKey);
    if (entry) {
      entry.value = Promise.resolve();
      entry.expiresAt = Date.now() + VERIFICATION_TTL_MS;
    }
    return;
  }

  if (mode === "stale-while-revalidate") {
    void scheduleVerificationRefresh(cacheKey, cached, url, integrity, label).catch(() => {});
    return cached.value;
  }

  const refresh = scheduleVerificationRefresh(cacheKey, cached, url, integrity, label);
  await refresh;

  const entry = verifiedUiCache.get(cacheKey);
  if (entry) {
    entry.value = Promise.resolve();
    entry.expiresAt = Date.now() + VERIFICATION_TTL_MS;
    entry.refreshing = undefined;
  }
}

async function verifyUiIntegrity(config: RuntimeConfig, mode: IntegrityVerificationMode) {
  if (!config.ui.url || !config.ui.integrity) {
    throw new TenantRuntimeError(
      "Tenant UI overrides must define app.ui.production and app.ui.integrity",
      404,
    );
  }

  await verifyIntegrity(config.ui.url, config.ui.integrity, `tenant UI ${config.ui.url}`, mode);
}

async function verifyPluginUiIntegrity(
  pluginKey: string,
  plugin: RuntimePlugin,
  mode: IntegrityVerificationMode,
) {
  if (!plugin.ui?.url) {
    throw new TenantRuntimeError(
      `Tenant plugin override for ${pluginKey} must define plugins.${pluginKey}.ui.production`,
      404,
    );
  }

  if (!plugin.ui.integrity) {
    throw new TenantRuntimeError(
      `Tenant plugin override for ${pluginKey} must define plugins.${pluginKey}.ui.integrity`,
      404,
    );
  }

  await verifyIntegrity(
    plugin.ui.url,
    plugin.ui.integrity,
    `tenant plugin UI ${pluginKey} ${plugin.ui.url}`,
    mode,
  );
}

function isPluginOverrideAllowed(
  allowedOverrides: ReadonlyArray<RuntimeOverrideTarget>,
  pluginKey: string,
): boolean {
  return (
    isRuntimeOverrideAllowed(allowedOverrides, "plugins") ||
    isRuntimeOverrideAllowed(allowedOverrides, `plugins.${pluginKey}`)
  );
}

function buildEffectivePluginConfig(
  basePlugin: RuntimePlugin,
  tenantPlugin: RuntimePlugin,
): RuntimePlugin {
  return {
    ...basePlugin,
    ...(tenantPlugin.sidebar ? { sidebar: tenantPlugin.sidebar } : {}),
    ...(tenantPlugin.ui ? { ui: tenantPlugin.ui } : {}),
  };
}

function buildEffectiveRuntimeConfig(
  baseConfig: RuntimeConfig,
  tenantConfig: RuntimeConfig,
  tenantAccountId: string,
  allowedOverrides: ReadonlyArray<RuntimeOverrideTarget>,
): RuntimeConfig {
  warnUnsupportedOverrideTargets(allowedOverrides);

  const effectiveConfig: RuntimeConfig = {
    ...baseConfig,
    account: tenantAccountId,
    networkId: tenantConfig.networkId,
    title: tenantConfig.title,
    description: tenantConfig.description,
    repository: tenantConfig.repository,
  };

  if (isRuntimeOverrideAllowed(allowedOverrides, "ui")) {
    effectiveConfig.ui = tenantConfig.ui;
  }

  const basePlugins = baseConfig.plugins ?? {};
  if (Object.keys(basePlugins).length > 0) {
    const effectivePlugins: NonNullable<RuntimeConfig["plugins"]> = { ...basePlugins };

    for (const [pluginKey, tenantPlugin] of Object.entries(tenantConfig.plugins ?? {})) {
      const basePlugin = basePlugins[pluginKey];
      if (!basePlugin) {
        continue;
      }

      if (!isPluginOverrideAllowed(allowedOverrides, pluginKey)) {
        continue;
      }

      effectivePlugins[pluginKey] = buildEffectivePluginConfig(basePlugin, tenantPlugin);
    }

    effectiveConfig.plugins = effectivePlugins;
  }

  return effectiveConfig;
}

function isSsrAllowed(accountId: string): boolean {
  if (parseBoolean(process.env.ALLOW_UNTRUSTED_SSR)) {
    return true;
  }

  return getTenantWhitelist().has(accountId);
}

export async function resolveRequestRuntime(
  baseConfig: RuntimeConfig,
  request: Request,
  options?: ResolveRequestRuntimeOptions,
): Promise<RequestRuntimeResolution> {
  const verificationMode = options?.verification ?? "blocking";
  const url = new URL(request.url);
  if (url.pathname.startsWith("/_runtime/")) {
    return {
      config: baseConfig,
      tenantAccountId: null,
      gatewayId: normalizeDomain(baseConfig.domain, baseConfig.host.url),
      ssrAllowed: Boolean(baseConfig.ui.ssrUrl),
    };
  }

  const gatewayId = normalizeDomain(baseConfig.domain, baseConfig.host.url);
  const tenantAccountId = resolveTenantAccountId(url.hostname, gatewayId, baseConfig.account);
  if (!tenantAccountId) {
    return {
      config: baseConfig,
      tenantAccountId: null,
      gatewayId,
      ssrAllowed: Boolean(baseConfig.ui.ssrUrl),
    };
  }

  const bosUrl = `bos://${tenantAccountId}/${gatewayId}`;
  const remoteConfig = await getRemoteConfigCached(bosUrl, "production");
  const baseBosUrl = `bos://${baseConfig.account}/${gatewayId}`;

  if (!remoteConfig.extendsChain.includes(baseBosUrl)) {
    throw new TenantRuntimeError(`Tenant config ${bosUrl} must extend ${baseBosUrl}`, 404);
  }

  if (remoteConfig.config.account !== tenantAccountId) {
    throw new TenantRuntimeError(
      `Tenant config ${bosUrl} resolved to account ${remoteConfig.config.account} instead of ${tenantAccountId}`,
      404,
    );
  }

  const tenantRuntimeConfig = buildRuntimeConfig(remoteConfig.config, process.cwd(), "production", {
    hostSource: "remote",
    uiSource: "remote",
    apiSource: "remote",
    authSource: "remote",
  });
  const effectiveConfig = buildEffectiveRuntimeConfig(
    baseConfig,
    tenantRuntimeConfig,
    tenantAccountId,
    getAllowedOverrides(),
  );

  if (effectiveConfig.ui.url !== baseConfig.ui.url) {
    await verifyUiIntegrity(effectiveConfig, verificationMode);
  }

  for (const [pluginKey, plugin] of Object.entries(effectiveConfig.plugins ?? {})) {
    const basePlugin = baseConfig.plugins?.[pluginKey];
    if (!basePlugin?.ui?.url || !plugin.ui?.url) {
      continue;
    }

    if (plugin.ui.url !== basePlugin.ui.url) {
      await verifyPluginUiIntegrity(pluginKey, plugin, verificationMode);
    }
  }

  const ssrAllowed =
    Boolean(effectiveConfig.ui.ssrUrl) &&
    Boolean(effectiveConfig.ui.ssrIntegrity) &&
    isSsrAllowed(tenantAccountId);

  return {
    config: ssrAllowed
      ? effectiveConfig
      : {
          ...effectiveConfig,
          ui: {
            ...effectiveConfig.ui,
            ssrUrl: undefined,
            ssrIntegrity: undefined,
          },
        },
    tenantAccountId,
    gatewayId,
    ssrAllowed,
  };
}
