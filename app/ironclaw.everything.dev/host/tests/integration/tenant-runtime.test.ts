import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RuntimeConfig } from "../../src/services/config";

const loadRemoteConfigMock = vi.fn();
const buildRuntimeConfigMock = vi.fn();
const verifySriForUrlMock = vi.fn();

vi.mock("everything-dev/config", async () => {
  return {
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

vi.mock("everything-dev/integrity", () => ({
  verifySriForUrl: verifySriForUrlMock,
}));

const { clearTenantRuntimeCaches, resolveRequestRuntime } = await import(
  "../../src/services/tenant-runtime"
);

function createDeferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

function createBaseRuntimeConfig(): RuntimeConfig {
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
      url: "https://linktree.com",
      entry: "https://linktree.com/mf-manifest.json",
      source: "remote",
    },
    ui: {
      name: "ui",
      url: "https://cdn.example.com/base-ui",
      entry: "https://cdn.example.com/base-ui/mf-manifest.json",
      source: "remote",
      integrity: "sha384-base",
      ssrUrl: "https://cdn.example.com/base-ui-ssr",
      ssrIntegrity: "sha384-base-ssr",
    },
    api: {
      name: "api",
      url: "https://api.example.com",
      entry: "https://api.example.com/mf-manifest.json",
      source: "remote",
    },
    auth: {
      name: "auth",
      url: "https://auth.example.com",
      entry: "https://auth.example.com/mf-manifest.json",
      source: "remote",
    },
    plugins: {
      apps: {
        name: "apps",
        url: "https://plugins.example.com/apps",
        entry: "https://plugins.example.com/apps/mf-manifest.json",
        source: "remote",
        sidebar: [{ icon: "Globe", label: "apps", to: "/apps", roleRequired: "anon" }],
        ui: {
          name: "apps-ui",
          url: "https://plugins.example.com/apps-ui",
          entry: "https://plugins.example.com/apps-ui/mf-manifest.json",
          source: "remote",
          integrity: "sha384-apps-base",
        },
      },
    },
  } as RuntimeConfig;
}

describe("resolveRequestRuntime", () => {
  const envSnapshot = { ...process.env };

  beforeEach(() => {
    vi.clearAllMocks();
    clearTenantRuntimeCaches();
    verifySriForUrlMock.mockResolvedValue(undefined);
    process.env = {
      ...envSnapshot,
      ALLOW_OVERRIDE: "ui",
      TENANT_WHITELIST: "alice.linktree.near",
      ALLOW_UNTRUSTED_SSR: "false",
    };
  });

  afterEach(() => {
    process.env = { ...envSnapshot };
  });

  it("returns the base runtime on the bare domain", async () => {
    const baseConfig = createBaseRuntimeConfig();

    const result = await resolveRequestRuntime(baseConfig, new Request("https://linktree.com/"));

    expect(result.config).toBe(baseConfig);
    expect(result.tenantAccountId).toBeNull();
    expect(loadRemoteConfigMock).not.toHaveBeenCalled();
  });

  it("derives tenant accounts relative to the active runtime account", async () => {
    loadRemoteConfigMock.mockResolvedValue({
      source: "bos://alice.linktree.near/linktree.com",
      rawConfig: {
        extends: "bos://linktree.near/linktree.com",
      },
      config: {
        account: "alice.linktree.near",
        app: {
          host: { development: "local:host", production: "https://host.example.com" },
          ui: { name: "ui", production: "https://cdn.example.com/alice-ui" },
          api: { name: "api", production: "https://api.example.com" },
        },
      },
      extendsChain: ["bos://alice.linktree.near/linktree.com", "bos://linktree.near/linktree.com"],
    });

    buildRuntimeConfigMock.mockReturnValue({
      ...createBaseRuntimeConfig(),
      account: "alice.linktree.near",
      ui: {
        ...createBaseRuntimeConfig().ui,
        url: "https://cdn.example.com/alice-ui",
        entry: "https://cdn.example.com/alice-ui/mf-manifest.json",
        integrity: "sha384-alice",
      },
    });

    const result = await resolveRequestRuntime(
      createBaseRuntimeConfig(),
      new Request("https://alice.linktree.com/"),
    );

    expect(result.tenantAccountId).toBe("alice.linktree.near");
    expect(loadRemoteConfigMock).toHaveBeenCalledWith(
      "bos://alice.linktree.near/linktree.com",
      "production",
    );
  });

  it("supports nested tenant labels within the active runtime namespace", async () => {
    loadRemoteConfigMock.mockResolvedValue({
      source: "bos://chicago.alice.linktree.near/linktree.com",
      rawConfig: {
        extends: "bos://linktree.near/linktree.com",
      },
      config: {
        account: "chicago.alice.linktree.near",
        app: {
          host: { development: "local:host", production: "https://host.example.com" },
          ui: { name: "ui", production: "https://cdn.example.com/chicago-ui" },
          api: { name: "api", production: "https://api.example.com" },
        },
      },
      extendsChain: [
        "bos://chicago.alice.linktree.near/linktree.com",
        "bos://linktree.near/linktree.com",
      ],
    });

    buildRuntimeConfigMock.mockReturnValue({
      ...createBaseRuntimeConfig(),
      account: "chicago.alice.linktree.near",
      ui: {
        ...createBaseRuntimeConfig().ui,
        url: "https://cdn.example.com/chicago-ui",
        entry: "https://cdn.example.com/chicago-ui/mf-manifest.json",
        integrity: "sha384-chicago",
      },
    });

    const result = await resolveRequestRuntime(
      createBaseRuntimeConfig(),
      new Request("https://chicago.alice.linktree.com/"),
    );

    expect(result.tenantAccountId).toBe("chicago.alice.linktree.near");
    expect(loadRemoteConfigMock).toHaveBeenCalledWith(
      "bos://chicago.alice.linktree.near/linktree.com",
      "production",
    );
  });

  it("requires the tenant config to extend the base runtime", async () => {
    loadRemoteConfigMock.mockResolvedValue({
      source: "bos://alice.linktree.near/linktree.com",
      rawConfig: {
        extends: "bos://somewhere-else.near/linktree.com",
      },
      config: {
        account: "alice.linktree.near",
        app: {
          host: { development: "local:host", production: "https://host.example.com" },
          ui: { name: "ui", production: "https://cdn.example.com/alice-ui" },
          api: { name: "api", production: "https://api.example.com" },
        },
      },
      extendsChain: [
        "bos://alice.linktree.near/linktree.com",
        "bos://somewhere-else.near/linktree.com",
      ],
    });

    buildRuntimeConfigMock.mockReturnValue(createBaseRuntimeConfig());

    await expect(
      resolveRequestRuntime(createBaseRuntimeConfig(), new Request("https://alice.linktree.com/")),
    ).rejects.toThrow("must extend bos://linktree.near/linktree.com");
  });

  it("applies a tenant UI override and allows SSR for whitelisted tenants", async () => {
    const baseConfig = createBaseRuntimeConfig();

    loadRemoteConfigMock.mockResolvedValue({
      source: "bos://alice.linktree.near/linktree.com",
      rawConfig: {
        extends: "bos://linktree.near/linktree.com",
      },
      config: {
        account: "alice.linktree.near",
        title: "Alice",
        description: "Alice links",
        repository: "https://github.com/example/alice",
        app: {
          host: { development: "local:host", production: "https://host.example.com" },
          ui: { name: "ui", production: "https://cdn.example.com/alice-ui" },
          api: { name: "api", production: "https://api.example.com" },
        },
      },
      extendsChain: ["bos://alice.linktree.near/linktree.com", "bos://linktree.near/linktree.com"],
    });

    buildRuntimeConfigMock.mockReturnValue({
      ...baseConfig,
      account: "alice.linktree.near",
      title: "Alice",
      description: "Alice links",
      repository: "https://github.com/example/alice",
      ui: {
        ...baseConfig.ui,
        url: "https://cdn.example.com/alice-ui",
        entry: "https://cdn.example.com/alice-ui/mf-manifest.json",
        integrity: "sha384-alice",
        ssrUrl: "https://cdn.example.com/alice-ui-ssr",
        ssrIntegrity: "sha384-alice-ssr",
      },
    });

    const result = await resolveRequestRuntime(
      baseConfig,
      new Request("https://alice.linktree.com/"),
    );

    expect(result.tenantAccountId).toBe("alice.linktree.near");
    expect(result.config.account).toBe("alice.linktree.near");
    expect(result.config.ui.url).toBe("https://cdn.example.com/alice-ui");
    expect(result.ssrAllowed).toBe(true);
    expect(result.config.ui.ssrUrl).toBe("https://cdn.example.com/alice-ui-ssr");
    expect(verifySriForUrlMock).toHaveBeenCalledWith(
      "https://cdn.example.com/alice-ui",
      "sha384-alice",
    );
  });

  it("disables SSR for non-whitelisted tenants when untrusted SSR is off", async () => {
    const baseConfig = createBaseRuntimeConfig();

    loadRemoteConfigMock.mockResolvedValue({
      source: "bos://bob.linktree.near/linktree.com",
      rawConfig: {
        extends: "bos://linktree.near/linktree.com",
      },
      config: {
        account: "bob.linktree.near",
        app: {
          host: { development: "local:host", production: "https://host.example.com" },
          ui: { name: "ui", production: "https://cdn.example.com/bob-ui" },
          api: { name: "api", production: "https://api.example.com" },
        },
      },
      extendsChain: ["bos://bob.linktree.near/linktree.com", "bos://linktree.near/linktree.com"],
    });

    buildRuntimeConfigMock.mockReturnValue({
      ...baseConfig,
      account: "bob.linktree.near",
      ui: {
        ...baseConfig.ui,
        url: "https://cdn.example.com/bob-ui",
        entry: "https://cdn.example.com/bob-ui/mf-manifest.json",
        integrity: "sha384-bob",
        ssrUrl: "https://cdn.example.com/bob-ui-ssr",
        ssrIntegrity: "sha384-bob-ssr",
      },
    });

    const result = await resolveRequestRuntime(
      baseConfig,
      new Request("https://bob.linktree.com/"),
    );

    expect(result.ssrAllowed).toBe(false);
    expect(result.config.ui.ssrUrl).toBeUndefined();
    expect(result.config.ui.ssrIntegrity).toBeUndefined();
  });

  it("disables SSR for whitelisted tenants when ssrIntegrity is missing", async () => {
    const baseConfig = createBaseRuntimeConfig();

    loadRemoteConfigMock.mockResolvedValue({
      source: "bos://alice.linktree.near/linktree.com",
      rawConfig: {
        extends: "bos://linktree.near/linktree.com",
      },
      config: {
        account: "alice.linktree.near",
        app: {
          host: { development: "local:host", production: "https://host.example.com" },
          ui: {
            name: "ui",
            production: "https://cdn.example.com/alice-ui",
            ssr: "https://cdn.example.com/alice-ui-ssr",
          },
          api: { name: "api", production: "https://api.example.com" },
        },
      },
      extendsChain: ["bos://alice.linktree.near/linktree.com", "bos://linktree.near/linktree.com"],
    });

    buildRuntimeConfigMock.mockReturnValue({
      ...baseConfig,
      account: "alice.linktree.near",
      ui: {
        name: "ui",
        url: "https://cdn.example.com/alice-ui",
        entry: "https://cdn.example.com/alice-ui/mf-manifest.json",
        source: "remote",
        integrity: "sha384-alice",
        ssrUrl: "https://cdn.example.com/alice-ui-ssr",
      },
    });

    const result = await resolveRequestRuntime(
      baseConfig,
      new Request("https://alice.linktree.com/"),
    );

    expect(result.ssrAllowed).toBe(false);
    expect(result.config.ui.ssrUrl).toBeUndefined();
    expect(result.config.ui.ssrIntegrity).toBeUndefined();
  });

  it("disables SSR for whitelisted tenants when ssrUrl is missing", async () => {
    const baseConfig = createBaseRuntimeConfig();

    loadRemoteConfigMock.mockResolvedValue({
      source: "bos://alice.linktree.near/linktree.com",
      rawConfig: {
        extends: "bos://linktree.near/linktree.com",
      },
      config: {
        account: "alice.linktree.near",
        app: {
          host: { development: "local:host", production: "https://host.example.com" },
          ui: { name: "ui", production: "https://cdn.example.com/alice-ui" },
          api: { name: "api", production: "https://api.example.com" },
        },
      },
      extendsChain: ["bos://alice.linktree.near/linktree.com", "bos://linktree.near/linktree.com"],
    });

    buildRuntimeConfigMock.mockReturnValue({
      ...baseConfig,
      account: "alice.linktree.near",
      ui: {
        name: "ui",
        url: "https://cdn.example.com/alice-ui",
        entry: "https://cdn.example.com/alice-ui/mf-manifest.json",
        source: "remote",
        integrity: "sha384-alice",
      },
    });

    const result = await resolveRequestRuntime(
      baseConfig,
      new Request("https://alice.linktree.com/"),
    );

    expect(result.ssrAllowed).toBe(false);
    expect(result.config.ui.ssrUrl).toBeUndefined();
    expect(result.config.ui.ssrIntegrity).toBeUndefined();
  });

  it("allows SSR for whitelisted tenants when both ssrUrl and ssrIntegrity are present", async () => {
    const baseConfig = createBaseRuntimeConfig();

    loadRemoteConfigMock.mockResolvedValue({
      source: "bos://alice.linktree.near/linktree.com",
      rawConfig: {
        extends: "bos://linktree.near/linktree.com",
      },
      config: {
        account: "alice.linktree.near",
        app: {
          host: { development: "local:host", production: "https://host.example.com" },
          ui: {
            name: "ui",
            production: "https://cdn.example.com/alice-ui",
            ssr: "https://cdn.example.com/alice-ui-ssr",
            ssrIntegrity: "sha384-alice-ssr",
          },
          api: { name: "api", production: "https://api.example.com" },
        },
      },
      extendsChain: ["bos://alice.linktree.near/linktree.com", "bos://linktree.near/linktree.com"],
    });

    buildRuntimeConfigMock.mockReturnValue({
      ...baseConfig,
      account: "alice.linktree.near",
      ui: {
        name: "ui",
        url: "https://cdn.example.com/alice-ui",
        entry: "https://cdn.example.com/alice-ui/mf-manifest.json",
        source: "remote",
        integrity: "sha384-alice",
        ssrUrl: "https://cdn.example.com/alice-ui-ssr",
        ssrIntegrity: "sha384-alice-ssr",
      },
    });

    const result = await resolveRequestRuntime(
      baseConfig,
      new Request("https://alice.linktree.com/"),
    );

    expect(result.ssrAllowed).toBe(true);
    expect(result.config.ui.ssrUrl).toBe("https://cdn.example.com/alice-ui-ssr");
    expect(result.config.ui.ssrIntegrity).toBe("sha384-alice-ssr");
  });

  it("applies existing plugin UI and sidebar overrides when plugins are allowed", async () => {
    const baseConfig = createBaseRuntimeConfig();
    process.env.ALLOW_OVERRIDE = "ui,plugins.*";

    loadRemoteConfigMock.mockResolvedValue({
      source: "bos://alice.linktree.near/linktree.com",
      rawConfig: {
        extends: "bos://linktree.near/linktree.com",
      },
      config: {
        account: "alice.linktree.near",
        app: {
          host: { development: "local:host", production: "https://host.example.com" },
          ui: { name: "ui", production: "https://cdn.example.com/alice-ui" },
          api: { name: "api", production: "https://api.example.com" },
        },
        plugins: {
          apps: {
            production: "https://plugins.example.com/alice-apps",
            ui: {
              production: "https://plugins.example.com/alice-apps-ui",
              integrity: "sha384-apps-alice",
            },
            sidebar: [{ icon: "Star", label: "alice apps", to: "/apps", roleRequired: "anon" }],
          },
          ignored: {
            production: "https://plugins.example.com/ignored",
            ui: {
              production: "https://plugins.example.com/ignored-ui",
              integrity: "sha384-ignored",
            },
          },
        },
      },
      extendsChain: ["bos://alice.linktree.near/linktree.com", "bos://linktree.near/linktree.com"],
    });

    buildRuntimeConfigMock.mockReturnValue({
      ...baseConfig,
      account: "alice.linktree.near",
      ui: {
        ...baseConfig.ui,
        url: "https://cdn.example.com/alice-ui",
        entry: "https://cdn.example.com/alice-ui/mf-manifest.json",
        integrity: "sha384-alice",
        ssrUrl: "https://cdn.example.com/alice-ui-ssr",
        ssrIntegrity: "sha384-alice-ssr",
      },
      plugins: {
        apps: {
          ...baseConfig.plugins!.apps,
          sidebar: [{ icon: "Star", label: "alice apps", to: "/apps", roleRequired: "anon" }],
          ui: {
            ...baseConfig.plugins!.apps.ui!,
            url: "https://plugins.example.com/alice-apps-ui",
            entry: "https://plugins.example.com/alice-apps-ui/mf-manifest.json",
            integrity: "sha384-apps-alice",
          },
        },
        ignored: {
          name: "ignored",
          url: "https://plugins.example.com/ignored",
          entry: "https://plugins.example.com/ignored/mf-manifest.json",
          source: "remote",
          ui: {
            name: "ignored-ui",
            url: "https://plugins.example.com/ignored-ui",
            entry: "https://plugins.example.com/ignored-ui/mf-manifest.json",
            source: "remote",
            integrity: "sha384-ignored",
          },
        },
      },
    });

    const result = await resolveRequestRuntime(
      baseConfig,
      new Request("https://alice.linktree.com/"),
    );

    expect(result.config.plugins?.apps.ui?.url).toBe("https://plugins.example.com/alice-apps-ui");
    expect(result.config.plugins?.apps.sidebar).toEqual([
      { icon: "Star", label: "alice apps", to: "/apps", roleRequired: "anon" },
    ]);
    expect(result.config.plugins?.ignored).toBeUndefined();
    expect(verifySriForUrlMock).toHaveBeenCalledWith(
      "https://plugins.example.com/alice-apps-ui",
      "sha384-apps-alice",
    );
  });

  it("revalidates expired tenant UI integrity in the background for stale asset requests", async () => {
    vi.useFakeTimers();

    try {
      const baseConfig = createBaseRuntimeConfig();

      loadRemoteConfigMock.mockResolvedValue({
        source: "bos://alice.linktree.near/linktree.com",
        rawConfig: {
          extends: "bos://linktree.near/linktree.com",
        },
        config: {
          account: "alice.linktree.near",
          app: {
            host: { development: "local:host", production: "https://host.example.com" },
            ui: { name: "ui", production: "https://cdn.example.com/alice-ui" },
            api: { name: "api", production: "https://api.example.com" },
          },
        },
        extendsChain: [
          "bos://alice.linktree.near/linktree.com",
          "bos://linktree.near/linktree.com",
        ],
      });

      buildRuntimeConfigMock.mockReturnValue({
        ...baseConfig,
        account: "alice.linktree.near",
        ui: {
          ...baseConfig.ui,
          url: "https://cdn.example.com/alice-ui",
          entry: "https://cdn.example.com/alice-ui/mf-manifest.json",
          integrity: "sha384-alice",
          ssrUrl: "https://cdn.example.com/alice-ui-ssr",
          ssrIntegrity: "sha384-alice-ssr",
        },
      });

      await resolveRequestRuntime(baseConfig, new Request("https://alice.linktree.com/"));

      const refresh = createDeferred<void>();
      verifySriForUrlMock.mockImplementationOnce(() => refresh.promise);
      vi.advanceTimersByTime(5 * 60_000 + 1);

      await expect(
        resolveRequestRuntime(baseConfig, new Request("https://alice.linktree.com/asset.js"), {
          verification: "stale-while-revalidate",
        }),
      ).resolves.toMatchObject({ tenantAccountId: "alice.linktree.near" });
      expect(verifySriForUrlMock).toHaveBeenCalledTimes(2);

      await expect(
        resolveRequestRuntime(baseConfig, new Request("https://alice.linktree.com/asset-2.js"), {
          verification: "stale-while-revalidate",
        }),
      ).resolves.toMatchObject({ tenantAccountId: "alice.linktree.near" });
      expect(verifySriForUrlMock).toHaveBeenCalledTimes(2);

      refresh.resolve();
      await Promise.resolve();
    } finally {
      vi.useRealTimers();
    }
  });

  it("waits for expired tenant UI integrity in blocking mode", async () => {
    vi.useFakeTimers();

    try {
      const baseConfig = createBaseRuntimeConfig();

      loadRemoteConfigMock.mockResolvedValue({
        source: "bos://alice.linktree.near/linktree.com",
        rawConfig: {
          extends: "bos://linktree.near/linktree.com",
        },
        config: {
          account: "alice.linktree.near",
          app: {
            host: { development: "local:host", production: "https://host.example.com" },
            ui: { name: "ui", production: "https://cdn.example.com/alice-ui" },
            api: { name: "api", production: "https://api.example.com" },
          },
        },
        extendsChain: [
          "bos://alice.linktree.near/linktree.com",
          "bos://linktree.near/linktree.com",
        ],
      });

      buildRuntimeConfigMock.mockReturnValue({
        ...baseConfig,
        account: "alice.linktree.near",
        ui: {
          ...baseConfig.ui,
          url: "https://cdn.example.com/alice-ui",
          entry: "https://cdn.example.com/alice-ui/mf-manifest.json",
          integrity: "sha384-alice",
          ssrUrl: "https://cdn.example.com/alice-ui-ssr",
          ssrIntegrity: "sha384-alice-ssr",
        },
      });

      await resolveRequestRuntime(baseConfig, new Request("https://alice.linktree.com/"));

      const refresh = createDeferred<void>();
      verifySriForUrlMock.mockImplementationOnce(() => refresh.promise);
      vi.advanceTimersByTime(5 * 60_000 + 1);

      let settled = false;
      const pending = resolveRequestRuntime(
        baseConfig,
        new Request("https://alice.linktree.com/"),
        {
          verification: "blocking",
        },
      ).then(() => {
        settled = true;
      });

      await Promise.resolve();
      expect(settled).toBe(false);

      refresh.resolve();
      await pending;
      expect(settled).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });

  it("recomputes the tenant whitelist when the env value changes", async () => {
    const baseConfig = createBaseRuntimeConfig();

    loadRemoteConfigMock.mockResolvedValue({
      source: "bos://bob.linktree.near/linktree.com",
      rawConfig: {
        extends: "bos://linktree.near/linktree.com",
      },
      config: {
        account: "bob.linktree.near",
        app: {
          host: { development: "local:host", production: "https://host.example.com" },
          ui: { name: "ui", production: "https://cdn.example.com/bob-ui" },
          api: { name: "api", production: "https://api.example.com" },
        },
      },
      extendsChain: ["bos://bob.linktree.near/linktree.com", "bos://linktree.near/linktree.com"],
    });

    buildRuntimeConfigMock.mockReturnValue({
      ...baseConfig,
      account: "bob.linktree.near",
      ui: {
        ...baseConfig.ui,
        url: "https://cdn.example.com/bob-ui",
        entry: "https://cdn.example.com/bob-ui/mf-manifest.json",
        integrity: "sha384-bob",
        ssrUrl: "https://cdn.example.com/bob-ui-ssr",
        ssrIntegrity: "sha384-bob-ssr",
      },
    });

    const blocked = await resolveRequestRuntime(
      baseConfig,
      new Request("https://bob.linktree.com/"),
    );
    expect(blocked.ssrAllowed).toBe(false);

    process.env.TENANT_WHITELIST = "bob.linktree.near";

    const allowed = await resolveRequestRuntime(
      baseConfig,
      new Request("https://bob.linktree.com/"),
    );
    expect(allowed.ssrAllowed).toBe(true);
  });
});
