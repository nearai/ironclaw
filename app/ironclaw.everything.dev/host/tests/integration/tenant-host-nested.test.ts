import { createServer } from "node:http";
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { getAvailablePort } from "../helpers/ports";

const loadRemoteConfigMock = vi.fn();
const buildRuntimeConfigMock = vi.fn();
const verifySriForUrlMock = vi.fn();

vi.mock("everything-dev/config", async () => {
  const actual =
    await vi.importActual<typeof import("everything-dev/config")>("everything-dev/config");
  return {
    ...actual,
    parseRuntimeOverrideTargets: (value?: string | null) =>
      value
        ? [
            ...new Set(
              value
                .split(",")
                .map((entry) => entry.trim())
                .filter(Boolean),
            ),
          ]
        : [],
    isRuntimeOverrideAllowed: (targets: string[], target: string) =>
      targets.includes(target) || (target.startsWith("plugins.") && targets.includes("plugins.*")),
    loadRemoteConfig: loadRemoteConfigMock,
    buildRuntimeConfig: buildRuntimeConfigMock,
  };
});

vi.mock("everything-dev/integrity", async () => {
  const actual = await vi.importActual<typeof import("everything-dev/integrity")>(
    "everything-dev/integrity",
  );
  return {
    ...actual,
    verifySriForUrl: verifySriForUrlMock,
  };
});

const { runServer } = await import("../../src/program");

function createBaseConfig() {
  return {
    env: "production",
    account: "linktree.near",
    domain: "linktree.com",
    networkId: "mainnet",
    title: "Linktree",
    description: "Base runtime",
    repository: "https://github.com/example/linktree",
    host: {
      name: "host",
      url: "http://127.0.0.1:0",
      entry: "http://127.0.0.1:0/mf-manifest.json",
      source: "remote",
    },
    ui: {
      name: "ui",
      url: "http://127.0.0.1:0/ui",
      entry: "http://127.0.0.1:0/ui/mf-manifest.json",
      source: "remote",
      integrity: "sha384-base",
    },
    api: {
      name: "api",
      url: "http://127.0.0.1:0/api",
      entry: "http://127.0.0.1:0/api/mf-manifest.json",
      source: "remote",
      proxy: "http://127.0.0.1:9",
    },
    plugins: {
      apps: {
        name: "apps",
        url: "http://127.0.0.1:0/apps",
        entry: "http://127.0.0.1:0/apps/mf-manifest.json",
        source: "remote",
        sidebar: [{ icon: "Globe", label: "apps", to: "/apps", roleRequired: "anon" }],
        ui: {
          name: "apps-ui",
          url: "http://127.0.0.1:0/apps-ui",
          entry: "http://127.0.0.1:0/apps-ui/mf-manifest.json",
          source: "remote",
          integrity: "sha384-apps",
        },
      },
    },
  } as const;
}

async function startStaticServer(routes: Record<string, { body: string; contentType?: string }>) {
  const port = await getAvailablePort();
  const server = createServer((req, res) => {
    const route = routes[req.url ?? ""];
    if (!route) {
      res.statusCode = 404;
      res.end("not found");
      return;
    }
    res.statusCode = 200;
    res.setHeader("content-type", route.contentType ?? "text/plain");
    res.end(route.body);
  });

  await new Promise<void>((resolve, reject) => {
    server.on("error", reject);
    server.listen(port, "127.0.0.1", () => resolve());
  });

  return {
    baseUrl: `http://127.0.0.1:${port}`,
    stop: async () => {
      await new Promise<void>((resolve, reject) => {
        server.close((error) => (error ? reject(error) : resolve()));
      });
    },
  };
}

describe("tenant host nested integration", () => {
  let assetServer: Awaited<ReturnType<typeof startStaticServer>>;
  let handle: ReturnType<typeof runServer>;
  let baseUrl: string;
  const previousNodeEnv = process.env.NODE_ENV;
  const previousHost = process.env.HOST;
  const previousPort = process.env.PORT;
  const previousAllowOverride = process.env.ALLOW_OVERRIDE;
  const previousTenantWhitelist = process.env.TENANT_WHITELIST;
  const previousAllowUntrustedSsr = process.env.ALLOW_UNTRUSTED_SSR;

  beforeAll(async () => {
    assetServer = await startStaticServer({
      "/__mf/plugin-ui/apps/chunk.js": {
        body: "console.log('nested-tenant-plugin-ui')",
        contentType: "application/javascript",
      },
    });

    const port = await getAvailablePort();
    baseUrl = `http://127.0.0.1:${port}`;
    process.env.NODE_ENV = "production";
    process.env.HOST = "127.0.0.1";
    process.env.PORT = String(port);
    process.env.ALLOW_OVERRIDE = "ui,plugins.*";
    process.env.TENANT_WHITELIST = "chicago.alice.linktree.near";
    process.env.ALLOW_UNTRUSTED_SSR = "false";

    const config = createBaseConfig();
    handle = runServer({
      config: {
        ...config,
        host: { ...config.host, url: baseUrl, entry: `${baseUrl}/mf-manifest.json` },
        ui: {
          ...config.ui,
          url: `${assetServer.baseUrl}/ui`,
          entry: `${assetServer.baseUrl}/ui/mf-manifest.json`,
        },
        api: { ...config.api, proxy: assetServer.baseUrl },
        plugins: {
          apps: {
            ...config.plugins.apps,
            ui: {
              ...config.plugins.apps.ui,
              url: `${assetServer.baseUrl}/apps-ui`,
              entry: `${assetServer.baseUrl}/apps-ui/mf-manifest.json`,
            },
          },
        },
      } as any,
    });

    await handle.ready;
  });

  afterAll(async () => {
    await handle?.shutdown();
    await assetServer?.stop();
    process.env.NODE_ENV = previousNodeEnv;
    process.env.HOST = previousHost;
    process.env.PORT = previousPort;
    process.env.ALLOW_OVERRIDE = previousAllowOverride;
    process.env.TENANT_WHITELIST = previousTenantWhitelist;
    process.env.ALLOW_UNTRUSTED_SSR = previousAllowUntrustedSsr;
  });

  beforeEach(() => {
    vi.clearAllMocks();
    verifySriForUrlMock.mockResolvedValue(undefined);
    const baseConfig = createBaseConfig();

    loadRemoteConfigMock.mockResolvedValue({
      source: "bos://chicago.alice.linktree.near/linktree.com",
      rawConfig: {
        extends: "bos://linktree.near/linktree.com",
      },
      config: {
        account: "chicago.alice.linktree.near",
        title: "Chicago Alice",
        description: "Nested tenant",
        repository: "https://github.com/example/chicago-alice",
        app: {
          host: { development: "local:host", production: "https://host.example.com" },
          ui: { name: "ui", production: "https://cdn.example.com/chicago-alice-ui" },
          api: { name: "api", production: "https://api.example.com" },
        },
        plugins: {
          apps: {
            production: "https://plugins.example.com/apps",
            ui: {
              production: "https://plugins.example.com/chicago-apps-ui",
              integrity: "sha384-chicago-apps",
            },
            sidebar: [{ icon: "Star", label: "chicago apps", to: "/apps", roleRequired: "anon" }],
          },
        },
      },
      extendsChain: [
        "bos://chicago.alice.linktree.near/linktree.com",
        "bos://linktree.near/linktree.com",
      ],
    });

    buildRuntimeConfigMock.mockReturnValue({
      ...baseConfig,
      account: "chicago.alice.linktree.near",
      title: "Chicago Alice",
      description: "Nested tenant",
      repository: "https://github.com/example/chicago-alice",
      ui: {
        ...baseConfig.ui,
        url: `${assetServer.baseUrl}/chicago-ui`,
        entry: `${assetServer.baseUrl}/chicago-ui/mf-manifest.json`,
        integrity: "sha384-chicago-ui",
      },
      api: {
        ...baseConfig.api,
        proxy: assetServer.baseUrl,
      },
      plugins: {
        apps: {
          ...baseConfig.plugins.apps,
          sidebar: [{ icon: "Star", label: "chicago apps", to: "/apps", roleRequired: "anon" }],
          ui: {
            ...baseConfig.plugins.apps.ui,
            url: assetServer.baseUrl,
            entry: `${assetServer.baseUrl}/apps-ui/mf-manifest.json`,
            integrity: "sha384-chicago-apps",
          },
        },
      },
    });
  });

  it("renders nested descendant UI remotes using the actual tenant resolver", async () => {
    const response = await fetch(`${baseUrl}/`, {
      headers: { "x-forwarded-host": "chicago.alice.linktree.com", "x-forwarded-proto": "https" },
    });

    const html = await response.text();

    expect(response.status).toBe(200);
    expect(loadRemoteConfigMock).toHaveBeenCalledWith(
      "bos://chicago.alice.linktree.near/linktree.com",
      "production",
    );
    expect(html).toContain(`${assetServer.baseUrl}/chicago-ui/remoteEntry.js`);
    expect(html).toContain(`/__mf/plugin-ui/apps/remoteEntry.js`);
  });
});
