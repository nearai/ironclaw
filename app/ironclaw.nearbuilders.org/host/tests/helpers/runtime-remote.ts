import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import type { BosConfig } from "everything-dev/types";
import { runServer } from "../../src/program";
import type { RuntimeConfig } from "../../src/services/config";

function getNetworkIdForAccount(account: string): string {
  return account.endsWith(".testnet") ? "testnet" : "mainnet";
}
import { startJsonProxyTarget } from "./json-proxy-target";
import { getAvailablePort } from "./ports";
import { loadHostTestEnv } from "./test-env";

export type RuntimeRemoteScenarioName = "remote-client" | "remote-ssr" | "remote-proxy";

export interface RuntimeRemoteScenario {
  name: RuntimeRemoteScenarioName;
  title: string;
  ssr: boolean;
  proxy: boolean;
  available: boolean;
  skipReason?: string;
}

export interface RuntimeRemoteHost {
  baseUrl: string;
  config: RuntimeConfig;
  stop: () => Promise<void>;
}

const currentDir = path.dirname(fileURLToPath(import.meta.url));
const workspaceRoot = path.resolve(currentDir, "../../..");
const bosConfigPath = path.join(workspaceRoot, "bos.config.json");

loadHostTestEnv(workspaceRoot);

function normalizeUrl(url: string) {
  return url.replace(/\/$/, "");
}

function toMfEntry(url: string) {
  return `${normalizeUrl(url)}/mf-manifest.json`;
}

async function loadRawBosConfig(): Promise<BosConfig> {
  const raw = await readFile(bosConfigPath, "utf8");
  return JSON.parse(raw) as BosConfig;
}

function getScenarioSkipReason(config: BosConfig, scenario: RuntimeRemoteScenarioName) {
  const uiProduction = config.app?.ui?.production;
  const uiSsr = config.app?.ui?.ssr;
  const apiProduction = config.app?.api?.production;

  if (!config.account) {
    return "Missing account in bos.config.json";
  }

  if (!uiProduction) {
    return "Missing app.ui.production in bos.config.json";
  }

  if (scenario !== "remote-proxy") {
    if (!apiProduction) {
      return "Missing app.api.production in bos.config.json";
    }
  }

  if (scenario === "remote-ssr") {
    if (!uiSsr) {
      return "Missing app.ui.ssr in bos.config.json";
    }
  }

  return undefined;
}

export async function getRuntimeRemoteScenarios(): Promise<RuntimeRemoteScenario[]> {
  const config = await loadRawBosConfig();
  const remoteClientSkipReason = getScenarioSkipReason(config, "remote-client");
  const remoteSsrSkipReason = getScenarioSkipReason(config, "remote-ssr");
  const remoteProxySkipReason = getScenarioSkipReason(config, "remote-proxy");

  return [
    {
      name: "remote-client",
      title: "remote ui + remote api without ssr",
      ssr: false,
      proxy: false,
      skipReason: remoteClientSkipReason,
      available: !remoteClientSkipReason,
    },
    {
      name: "remote-ssr",
      title: "remote ui + remote api with ssr",
      ssr: true,
      proxy: false,
      skipReason: remoteSsrSkipReason,
      available: !remoteSsrSkipReason,
    },
    {
      name: "remote-proxy",
      title: "remote ui + proxy api without ssr",
      ssr: false,
      proxy: true,
      skipReason: remoteProxySkipReason,
      available: !remoteProxySkipReason,
    },
  ];
}

function buildRuntimeConfig(
  config: BosConfig,
  scenario: RuntimeRemoteScenario,
  hostUrl: string,
  proxyTargetUrl?: string,
): RuntimeConfig {
  const uiUrl = config.app?.ui?.production;
  const apiUrl = scenario.proxy ? proxyTargetUrl : (config.app?.api?.production ?? "");

  if (!config.account || !uiUrl) {
    throw new Error(`Scenario ${scenario.name} is missing required remote config`);
  }

  return {
    env: "development",
    account: config.account,
    networkId: getNetworkIdForAccount(config.account),
    title: config.account,
    repository: config.repository,
    host: {
      name: "host",
      url: hostUrl,
      entry: `${hostUrl}/mf-manifest.json`,
      source: "remote" as const,
    },
    ui: {
      name: config.app?.ui?.name ?? "ui",
      url: normalizeUrl(uiUrl),
      entry: toMfEntry(uiUrl),
      source: "remote",
      ssrUrl: scenario.ssr ? config.app?.ui?.ssr : undefined,
    },
    api: {
      name: config.app?.api?.name ?? "api",
      url: apiUrl ? normalizeUrl(apiUrl) : "",
      entry: apiUrl ? toMfEntry(apiUrl) : "",
      source: "remote",
      proxy: scenario.proxy && proxyTargetUrl ? normalizeUrl(proxyTargetUrl) : undefined,
      variables: config.app?.api?.variables,
      secrets: config.app?.api?.secrets,
      shared: config.app?.api?.shared,
    },
  } as RuntimeConfig;
}

export async function startRuntimeRemoteHost(
  scenario: RuntimeRemoteScenario,
): Promise<RuntimeRemoteHost> {
  if (!scenario.available) {
    throw new Error(scenario.skipReason ?? `Scenario ${scenario.name} is unavailable`);
  }

  const rawConfig = await loadRawBosConfig();
  const port = await getAvailablePort();
  const baseUrl = `http://127.0.0.1:${port}`;
  const proxyTarget = scenario.proxy ? await startJsonProxyTarget() : null;
  const runtimeConfig = buildRuntimeConfig(rawConfig, scenario, baseUrl, proxyTarget?.baseUrl);

  const previousNodeEnv = process.env.NODE_ENV;
  const previousHost = process.env.HOST;
  const previousPort = process.env.PORT;
  process.env.NODE_ENV = "development";
  process.env.HOST = "127.0.0.1";
  process.env.PORT = String(port);

  const handle = runServer({ config: runtimeConfig });

  try {
    await handle.ready;
  } catch (error) {
    await handle.shutdown().catch(() => undefined);
    await proxyTarget?.stop().catch(() => undefined);
    process.env.NODE_ENV = previousNodeEnv;
    process.env.HOST = previousHost;
    process.env.PORT = previousPort;
    throw error;
  }

  return {
    baseUrl,
    config: runtimeConfig,
    stop: async () => {
      await handle.shutdown();
      await proxyTarget?.stop();
      process.env.NODE_ENV = previousNodeEnv;
      process.env.HOST = previousHost;
      process.env.PORT = previousPort;
    },
  };
}
