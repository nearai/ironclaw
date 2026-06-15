import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { createInstance } from "@module-federation/enhanced/runtime";
import { setGlobalFederationInstance } from "@module-federation/runtime-core";
import { patchManifestFetchForSsrPublicPath } from "everything-dev/mf";
import type { RuntimeConfig } from "../../src/services/config";
import { getAvailablePort } from "./ports";
import { startStaticDistServer } from "./static-dist-server";
import { loadHostTestEnv } from "./test-env";

interface ServerHandle {
  ready: Promise<void>;
  shutdown: () => Promise<void>;
}

interface HostRemoteModule {
  runServer: (input: { config: RuntimeConfig }) => ServerHandle;
}

export interface BundledHostRuntime {
  baseUrl: string;
  hostAssetsUrl: string;
  uiAssetsUrl: string;
  stop: () => Promise<void>;
}

export interface BundledHostUrls {
  baseUrl: string;
  hostAssetsUrl: string;
  uiAssetsUrl: string;
}

const workspaceRoot = path.resolve(import.meta.dirname, "../../..");
const hostDir = path.join(workspaceRoot, "host");
const uiDir = path.join(workspaceRoot, "ui");
let buildReady = false;

loadHostTestEnv(workspaceRoot);

function ensureBuild(cwd: string) {
  const result = spawnSync("bun", ["run", "build"], {
    cwd,
    stdio: "inherit",
    env: { ...process.env },
  });

  if (result.status !== 0) {
    throw new Error(`Build failed for ${cwd} (exit ${result.status ?? "unknown"})`);
  }
}

function ensureBuilds() {
  if (buildReady) return;

  if (!existsSync(path.join(hostDir, "dist", "remoteEntry.js"))) {
    ensureBuild(hostDir);
  }
  if (!existsSync(path.join(uiDir, "dist", "remoteEntry.server.js"))) {
    ensureBuild(uiDir);
  }
  buildReady = true;
}

let instanceCounter = 0;

async function loadBundledHostModule(hostAssetsUrl: string): Promise<HostRemoteModule> {
  instanceCounter++;
  const remoteName = `host_${instanceCounter}`;
  const mf = createInstance({ name: `bundled-host-${instanceCounter}`, remotes: [] });
  setGlobalFederationInstance(mf as any);
  patchManifestFetchForSsrPublicPath(mf as any);

  const entryUrl = `${hostAssetsUrl}/mf-manifest.json`;
  (mf as any).registerRemotes([{ name: remoteName, entry: entryUrl, alias: "host" }]);

  const hostModule = (await (mf as any).loadRemote(
    `${remoteName}/Server`,
  )) as HostRemoteModule | null;
  if (!hostModule?.runServer) {
    throw new Error("Bundled host module did not export runServer");
  }

  return hostModule;
}

export async function startBundledHost(
  buildConfig: (urls: BundledHostUrls) => RuntimeConfig,
): Promise<BundledHostRuntime> {
  ensureBuilds();

  const hostAssetsServer = await startStaticDistServer(path.join(hostDir, "dist"));
  const uiAssetsServer = await startStaticDistServer(path.join(uiDir, "dist"));
  const port = await getAvailablePort();
  const baseUrl = `http://127.0.0.1:${port}`;
  const runtimeConfig = buildConfig({
    baseUrl,
    hostAssetsUrl: hostAssetsServer.baseUrl,
    uiAssetsUrl: uiAssetsServer.baseUrl,
  });

  const previousNodeEnv = process.env.NODE_ENV;
  const previousHost = process.env.HOST;
  const previousPort = process.env.PORT;
  process.env.NODE_ENV = "production";
  process.env.HOST = "127.0.0.1";
  process.env.PORT = String(port);

  let serverHandle: ServerHandle | null = null;

  try {
    const hostModule = await loadBundledHostModule(hostAssetsServer.baseUrl);
    serverHandle = hostModule.runServer({ config: runtimeConfig });
    await serverHandle.ready;
  } catch (error) {
    await Promise.allSettled([hostAssetsServer.stop(), uiAssetsServer.stop()]);
    process.env.NODE_ENV = previousNodeEnv;
    process.env.HOST = previousHost;
    process.env.PORT = previousPort;
    throw error;
  }

  return {
    baseUrl,
    hostAssetsUrl: hostAssetsServer.baseUrl,
    uiAssetsUrl: uiAssetsServer.baseUrl,
    stop: async () => {
      await serverHandle?.shutdown();
      await Promise.allSettled([hostAssetsServer.stop(), uiAssetsServer.stop()]);
      process.env.NODE_ENV = previousNodeEnv;
      process.env.HOST = previousHost;
      process.env.PORT = previousPort;
    },
  };
}
