import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { createInstance } from "@module-federation/enhanced/runtime";
import { setGlobalFederationInstance } from "@module-federation/runtime-core";
import { patchManifestFetchForSsrPublicPath } from "everything-dev/mf";
import type { RouterModule } from "../../src/types";
import { type StaticDistServer, startStaticDistServer } from "./static-dist-server";
import { loadHostTestEnv } from "./test-env";

const workspaceRoot = path.resolve(import.meta.dirname, "../../..");
const uiDir = path.join(workspaceRoot, "ui");
let buildReady = false;

loadHostTestEnv(workspaceRoot);

function ensureUiServerBuild() {
  if (buildReady) return;

  const serverEntry = path.join(uiDir, "dist", "remoteEntry.server.js");
  if (existsSync(serverEntry)) {
    buildReady = true;
    return;
  }

  const result = spawnSync("bun", ["run", "build"], {
    cwd: uiDir,
    stdio: "inherit",
    env: { ...process.env, BUILD_TARGET: "server" },
  });

  if (result.status !== 0) {
    throw new Error(`UI server build failed (exit ${result.status ?? "unknown"})`);
  }

  buildReady = true;
}

let activeSsrLoader: {
  uiServer: StaticDistServer;
  mf: ReturnType<typeof createInstance>;
} | null = null;

export async function loadBundledRouterModule(): Promise<{
  routerModule: RouterModule;
  assetsUrl: string;
  cleanup: () => Promise<void>;
}> {
  ensureUiServerBuild();

  if (activeSsrLoader) {
    const mod = await (activeSsrLoader.mf as any).loadRemote("ui/Router", { from: "build" });
    return {
      routerModule: mod.default as RouterModule,
      assetsUrl: activeSsrLoader.uiServer.baseUrl,
      cleanup: async () => {},
    };
  }

  const uiServer = await startStaticDistServer(path.join(uiDir, "dist"));

  const mf = createInstance({
    name: "ssr-test-host",
    remotes: [
      {
        name: "ui",
        entry: `${uiServer.baseUrl}/mf-manifest.json`,
        alias: "ui",
      },
    ],
  });
  setGlobalFederationInstance(mf as any);
  patchManifestFetchForSsrPublicPath(mf as any);

  const mod = await (mf as any).loadRemote("ui/Router", { from: "build" });
  if (!mod?.default) {
    await uiServer.stop();
    throw new Error("Bundled UI remote did not export Router module");
  }

  activeSsrLoader = { uiServer, mf };

  return {
    routerModule: mod.default as RouterModule,
    assetsUrl: uiServer.baseUrl,
    cleanup: async () => {
      if (activeSsrLoader) {
        await activeSsrLoader.uiServer.stop();
        if ((mf as any).getInstance) {
          setGlobalFederationInstance(undefined as any);
        }
        activeSsrLoader = null;
      }
    },
  };
}
