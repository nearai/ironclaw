import { Effect } from "every-plugin/effect";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RuntimeConfig } from "../../src/services/config";

const loadRemoteMock = vi.fn();
const createInstanceMock = vi.fn(() => ({
  loadRemote: loadRemoteMock,
}));
const verifySriForUrlMock = vi.fn();

vi.mock("@module-federation/enhanced/runtime", () => ({
  createInstance: createInstanceMock,
}));

vi.mock("everything-dev/integrity", () => ({
  verifySriForUrl: verifySriForUrlMock,
}));

const { loadRouterModule, resetFederationInstance } = await import(
  "../../src/services/federation.server"
);

const { FederationError } = await import("../../src/services/errors");

function createRemoteConfig(options?: { ssrUrl?: string; ssrIntegrity?: string }): RuntimeConfig {
  return {
    env: "production",
    account: "linktree.near",
    networkId: "mainnet",
    host: {
      name: "host",
      url: "https://linktree.com",
      entry: "https://linktree.com/mf-manifest.json",
      source: "remote",
    },
    ui: {
      name: "ui",
      url: "https://cdn.example.com/ui",
      entry: "https://cdn.example.com/ui/mf-manifest.json",
      source: "remote",
      integrity: "sha384-ui",
      ssrUrl: options?.ssrUrl ?? "https://cdn.example.com/ui-ssr",
      ssrIntegrity: options?.ssrIntegrity ?? "sha384-ssr-a",
    },
    api: {
      name: "api",
      url: "https://api.example.com",
      entry: "https://api.example.com/mf-manifest.json",
      source: "remote",
    },
  } as RuntimeConfig;
}

describe("loadRouterModule failure paths", () => {
  beforeEach(() => {
    resetFederationInstance();
    vi.clearAllMocks();
    verifySriForUrlMock.mockResolvedValue(undefined);
  });

  describe("loadRemote returns null", () => {
    it("raises FederationError with Module not found message", async () => {
      loadRemoteMock.mockResolvedValue(null);

      const result = await Effect.runPromise(
        loadRouterModule(createRemoteConfig()).pipe(Effect.either),
      );

      expect(result._tag).toBe("Left");
      if (result._tag !== "Left") throw new Error("Expected Left");
      const error = result.left as InstanceType<typeof FederationError>;
      expect(error._tag).toBe("FederationError");
      expect(error.remoteName).toBe("ui");
      expect(String(error.cause)).toContain("Module not found: ui/Router");
    });
  });

  describe("loadRemote throws a Module Federation runtime error", () => {
    it("wraps the MF error in FederationError", async () => {
      const mfError = new Error("An error has occurred");
      mfError.name = "FederationError";
      loadRemoteMock.mockRejectedValue(mfError);

      const result = await Effect.runPromise(
        loadRouterModule(createRemoteConfig()).pipe(Effect.either),
      );

      expect(result._tag).toBe("Left");
      if (result._tag !== "Left") throw new Error("Expected Left");
      const error = result.left as InstanceType<typeof FederationError>;
      expect(error._tag).toBe("FederationError");
      expect(error.remoteName).toBe("ui");
      expect(error.cause).toBeDefined();
      expect(String(error.cause)).toContain("An error has occurred");
    });
  });

  describe("SRI verification fails", () => {
    it("raises FederationError wrapping the integrity error", async () => {
      const integrityError = new Error("SRI hash mismatch: expected sha384-abc, got sha384-xyz");
      verifySriForUrlMock.mockRejectedValue(integrityError);

      const result = await Effect.runPromise(
        loadRouterModule(createRemoteConfig()).pipe(Effect.either),
      );

      expect(result._tag).toBe("Left");
      if (result._tag !== "Left") throw new Error("Expected Left");
      const error = result.left as InstanceType<typeof FederationError>;
      expect(error._tag).toBe("FederationError");
      expect(error.remoteUrl).toBe("https://cdn.example.com/ui-ssr");
    });
  });

  describe("SSR URL missing in production", () => {
    it("raises error when ssrUrl is not configured", async () => {
      const config = createRemoteConfig({ ssrUrl: undefined });

      const result = await Effect.runPromise(loadRouterModule(config).pipe(Effect.either));

      expect(result._tag).toBe("Left");
    });
  });

  describe("retry succeeds after initial failures", () => {
    it("retries up to 5 times and succeeds on the third attempt", async () => {
      const routerModule = {
        default: { renderToStream: vi.fn(), getRouteHead: vi.fn(), createRouter: vi.fn() },
      };

      loadRemoteMock
        .mockRejectedValueOnce(new Error("Network error"))
        .mockRejectedValueOnce(new Error("Timeout"))
        .mockResolvedValue(routerModule);

      const result = await Effect.runPromise(loadRouterModule(createRemoteConfig()));

      expect(result).toBe(routerModule.default);
      expect(createInstanceMock).toHaveBeenCalledTimes(3);
    });
  });

  describe("retry exhausts all 5 attempts", () => {
    it("raises FederationError after all retries fail", async () => {
      loadRemoteMock.mockRejectedValue(new Error("Persistent failure"));

      const result = await Effect.runPromise(
        loadRouterModule(createRemoteConfig()).pipe(Effect.either),
      );

      expect(result._tag).toBe("Left");
      if (result._tag !== "Left") throw new Error("Expected Left");
      const error = result.left as InstanceType<typeof FederationError>;
      expect(error._tag).toBe("FederationError");
      expect(createInstanceMock).toHaveBeenCalledTimes(6);
    });
  });

  describe("cache evicts on failure", () => {
    it("removes the cache entry when the load promise rejects", { timeout: 30000 }, async () => {
      const config = createRemoteConfig();

      loadRemoteMock.mockRejectedValue(new Error("Load failed"));

      const result = await Effect.runPromise(loadRouterModule(config).pipe(Effect.either));
      expect(result._tag).toBe("Left");

      loadRemoteMock.mockRejectedValue(new Error("Still failing"));
      const secondResult = await Effect.runPromise(loadRouterModule(config).pipe(Effect.either));
      expect(secondResult._tag).toBe("Left");

      expect(createInstanceMock.mock.calls.length).toBeGreaterThanOrEqual(2);
    });

    it("recovers after cache eviction on a subsequent request", { timeout: 30000 }, async () => {
      const config = createRemoteConfig();

      loadRemoteMock.mockRejectedValue(new Error("Load failed"));

      const firstResult = await Effect.runPromise(loadRouterModule(config).pipe(Effect.either));
      expect(firstResult._tag).toBe("Left");

      const routerModule = {
        default: { renderToStream: vi.fn(), getRouteHead: vi.fn(), createRouter: vi.fn() },
      };
      loadRemoteMock.mockResolvedValue(routerModule);

      const secondResult = await Effect.runPromise(loadRouterModule(config));
      expect(secondResult).toBe(routerModule.default);
    });
  });

  describe("FederationError structure matches production error", () => {
    it("includes remoteName and remoteUrl in the error", async () => {
      loadRemoteMock.mockResolvedValue(null);

      const result = await Effect.runPromise(
        loadRouterModule(createRemoteConfig()).pipe(Effect.either),
      );

      expect(result._tag).toBe("Left");
      if (result._tag !== "Left") throw new Error("Expected Left");
      const error = result.left as InstanceType<typeof FederationError>;
      expect(error).toHaveProperty("remoteName", "ui");
      expect(error).toHaveProperty("remoteUrl", "https://cdn.example.com/ui-ssr");
    });

    it("cause contains the original MF error that matches the stack trace pattern", async () => {
      const mfError = new Error("An error has occurred");
      mfError.stack = [
        "Error: An error has occurred",
        "    at catch (file:///remoteEntry.js:286:77)",
        "    at effect_internal_function (/app/node_modules/effect/dist/esm/Utils.js:339:14)",
      ].join("\n");
      loadRemoteMock.mockRejectedValue(mfError);

      const result = await Effect.runPromise(
        loadRouterModule(createRemoteConfig()).pipe(Effect.either),
      );

      expect(result._tag).toBe("Left");
      if (result._tag !== "Left") throw new Error("Expected Left");
      const error = result.left as InstanceType<typeof FederationError>;
      expect(error.cause).toBeDefined();
      const causeStr = String(error.cause);
      expect(causeStr).toContain("remoteEntry.js");
    });
  });
});
