import { createServer } from "node:http";
import { Effect } from "every-plugin/effect";
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { getAvailablePort } from "../helpers/ports";

const loadRouterModuleMock = vi.fn();
const resolveRequestRuntimeMock = vi.fn();

vi.mock("../../src/services/federation.server", async () => {
  const actual = await vi.importActual<typeof import("../../src/services/federation.server")>(
    "../../src/services/federation.server",
  );
  return {
    ...actual,
    loadRouterModule: loadRouterModuleMock,
  };
});

vi.mock("../../src/services/tenant-runtime", async () => {
  const actual = await vi.importActual<typeof import("../../src/services/tenant-runtime")>(
    "../../src/services/tenant-runtime",
  );
  return {
    ...actual,
    resolveRequestRuntime: resolveRequestRuntimeMock,
  };
});

const { FederationError } = await import("../../src/services/errors");
const { runServer } = await import("../../src/program");

function createBaseConfig(ssrUrl?: string) {
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
      ssrUrl,
    },
    api: {
      name: "api",
      url: "http://127.0.0.1:0/api",
      entry: "http://127.0.0.1:0/api/mf-manifest.json",
      source: "remote",
      proxy: "http://127.0.0.1:9",
    },
    plugins: {},
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

describe("SSR fallback paths", () => {
  let assetServer: Awaited<ReturnType<typeof startStaticServer>>;
  let handle: ReturnType<typeof runServer>;
  let baseUrl: string;
  const envSnapshot = { ...process.env };

  beforeAll(async () => {
    assetServer = await startStaticServer({});

    const port = await getAvailablePort();
    baseUrl = `http://127.0.0.1:${port}`;
    process.env.NODE_ENV = "production";
    process.env.HOST = "127.0.0.1";
    process.env.PORT = String(port);
    process.env.CSP_STRICT = "false";

    const config = createBaseConfig(`${assetServer.baseUrl}/ui-ssr`);
    handle = runServer({
      config: {
        ...config,
        host: { ...config.host, url: baseUrl, entry: `${baseUrl}/mf-manifest.json` },
        ui: {
          ...config.ui,
          url: `${assetServer.baseUrl}/ui`,
          entry: `${assetServer.baseUrl}/ui/mf-manifest.json`,
          ssrUrl: `${assetServer.baseUrl}/ui-ssr`,
        },
        api: { ...config.api, proxy: assetServer.baseUrl },
      } as any,
    });

    await handle.ready;
  });

  afterAll(async () => {
    await handle?.shutdown();
    await assetServer?.stop();
    process.env = { ...envSnapshot };
  });

  beforeEach(() => {
    loadRouterModuleMock.mockReset();
    resolveRequestRuntimeMock.mockReset();

    resolveRequestRuntimeMock.mockResolvedValue({
      tenantAccountId: null,
      gatewayId: "linktree.com",
      ssrAllowed: true,
      config: createBaseConfig(`${assetServer.baseUrl}/ui-ssr`),
    });
  });

  describe("loadRouterModule fails with FederationError", () => {
    it("falls back to client shell with SSR unavailable message", async () => {
      const federationError = new FederationError({
        remoteName: "ui",
        remoteUrl: `${assetServer.baseUrl}/ui-ssr`,
        cause: new Error("An error has occurred"),
      });

      loadRouterModuleMock.mockReturnValue(
        Effect.gen(function* () {
          return yield* Effect.fail(federationError);
        }),
      );

      const response = await fetch(`${baseUrl}/`);
      const html = await response.text();

      expect(response.status).toBe(200);
      expect(html).toContain("SSR unavailable");
      expect(html).toContain("remoteEntry.js");
      expect(html).toContain("window.__RUNTIME_CONFIG__");
    });

    it("includes the FederationError cause message in the client shell fallback", async () => {
      const federationError = new FederationError({
        remoteName: "ui",
        remoteUrl: `${assetServer.baseUrl}/ui-ssr`,
        cause: new Error("Module not found: ui/Router"),
      });

      loadRouterModuleMock.mockReturnValue(
        Effect.gen(function* () {
          return yield* Effect.fail(federationError);
        }),
      );

      const response = await fetch(`${baseUrl}/`);
      const html = await response.text();

      expect(response.status).toBe(200);
      expect(html).toContain("SSR unavailable");

      const errorParagraphMatch = html.match(
        /<p class="error">SSR unavailable, showing client app\.<\/p><p>(.*?)<\/p>/,
      );
      expect(errorParagraphMatch).not.toBeNull();
      expect(errorParagraphMatch![1].length).toBeGreaterThan(0);
    });

    it("preserves FederationError context for logging", async () => {
      const cause = new Error("An error has occurred");
      const federationError = new FederationError({
        remoteName: "ui",
        remoteUrl: `${assetServer.baseUrl}/ui-ssr`,
        cause,
      });

      loadRouterModuleMock.mockReturnValue(
        Effect.gen(function* () {
          return yield* Effect.fail(federationError);
        }),
      );

      await fetch(`${baseUrl}/`);

      expect(loadRouterModuleMock).toHaveBeenCalled();
    });
  });

  describe("SSR URL not configured", () => {
    it("renders client shell without attempting SSR", async () => {
      resolveRequestRuntimeMock.mockResolvedValue({
        tenantAccountId: null,
        gatewayId: "linktree.com",
        ssrAllowed: false,
        config: createBaseConfig(undefined),
      });

      const response = await fetch(`${baseUrl}/`);
      const html = await response.text();

      expect(response.status).toBe(200);
      expect(html).toContain("Loading...");
      expect(html).toContain("remoteEntry.js");
      expect(loadRouterModuleMock).not.toHaveBeenCalled();
    });
  });

  describe("renderToStream throws", () => {
    it("returns 500 error page", async () => {
      const failingModule = {
        renderToStream: vi.fn().mockRejectedValue(new Error("React render error")),
        getRouteHead: vi.fn(),
        createRouter: vi.fn(),
      };

      loadRouterModuleMock.mockReturnValue(Effect.succeed(failingModule));

      const response = await fetch(`${baseUrl}/`);
      const html = await response.text();

      expect(response.status).toBe(500);
      expect(html).toContain("Server Error");
      expect(html).toContain("React render error");
    });
  });

  describe("SSR succeeds after previous failure", () => {
    it("recovers from a transient FederationError on the next request", async () => {
      const federationError = new FederationError({
        remoteName: "ui",
        remoteUrl: `${assetServer.baseUrl}/ui-ssr`,
        cause: new Error("Transient network error"),
      });

      loadRouterModuleMock.mockReturnValueOnce(
        Effect.gen(function* () {
          return yield* Effect.fail(federationError);
        }),
      );

      const firstResponse = await fetch(`${baseUrl}/`);
      const firstHtml = await firstResponse.text();
      expect(firstResponse.status).toBe(200);
      expect(firstHtml).toContain("SSR unavailable");

      const stream = new ReadableStream({
        start(controller) {
          controller.enqueue(
            new TextEncoder().encode(
              '<!DOCTYPE html><html><head><title>SSR Page</title></head><body><div id="root">SSR Content</div></body></html>',
            ),
          );
          controller.close();
        },
      });

      const successModule = {
        renderToStream: vi.fn().mockResolvedValue({
          stream,
          statusCode: 200,
          headers: new Headers(),
        }),
        getRouteHead: vi.fn(),
        createRouter: vi.fn(),
      };

      loadRouterModuleMock.mockReturnValueOnce(Effect.succeed(successModule));

      const secondResponse = await fetch(`${baseUrl}/`);
      expect(secondResponse.status).toBe(200);
    });
  });

  describe("FederationError message propagation bug", () => {
    it("FederationError.message is empty when passed through Effect.either", async () => {
      const federationError = new FederationError({
        remoteName: "ui",
        remoteUrl: `${assetServer.baseUrl}/ui-ssr`,
        cause: new Error("An error has occurred"),
      });

      expect(federationError._tag).toBe("FederationError");
      expect(federationError.remoteName).toBe("ui");
      expect(federationError.cause).toBeDefined();

      const result = await Effect.runPromise(Effect.fail(federationError).pipe(Effect.either));

      expect(result._tag).toBe("Left");
      if (result._tag !== "Left") throw new Error("Expected Left");
      const leftError = result.left as InstanceType<typeof FederationError>;
      expect(leftError._tag).toBe("FederationError");

      expect(leftError.message.length).toBeGreaterThan(0);
    });
  });
});
