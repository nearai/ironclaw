import path from "node:path";
import { startRebornMock } from "../../../tests/reborn-mock/server";
import type { ScenarioName } from "../../../tests/reborn-mock/types";
import { type BundledHostUrls, startBundledHost } from "./bundled-host";
import { loadHostTestEnv } from "./test-env";

const workspaceRoot = path.resolve(import.meta.dirname, "../../..");

loadHostTestEnv(workspaceRoot);

export interface RebornAppHost {
  baseUrl: string;
  rebornBaseUrl: string;
  rebornToken: string;
  stop: () => Promise<void>;
  setRebornScenario: (name: ScenarioName) => void;
}

export async function startRebornApp(): Promise<RebornAppHost> {
  const mock = await startRebornMock({ scenario: "healthy-empty" });

  const host = await startBundledHost((urls: BundledHostUrls) => {
    return {
      env: "production" as const,
      account: "test.near",
      networkId: "testnet" as const,
      title: "Test Host",
      repository: "https://github.com/test/repo",
      host: {
        name: "host",
        url: urls.baseUrl,
        entry: `${urls.hostAssetsUrl}/mf-manifest.json`,
        source: "local" as const,
      },
      ui: {
        name: "ui",
        url: urls.uiAssetsUrl,
        entry: `${urls.uiAssetsUrl}/mf-manifest.json`,
        source: "local" as const,
      },
      api: {
        name: "api",
        url: urls.baseUrl,
        entry: "",
        source: "local" as const,
      },
    };
  });

  return {
    baseUrl: host.baseUrl,
    rebornBaseUrl: mock.baseUrl,
    rebornToken: mock.token,
    stop: async () => {
      await mock.stop();
      await host.stop();
    },
    setRebornScenario: (name: ScenarioName) => mock.setScenario(name),
  };
}
