import { type Context, Hono } from "hono";
import { afterAll, beforeAll, beforeEach, describe, expect, it, type Mock, vi } from "vitest";
import { proxyRequest, setupApiRoutes } from "../../src/program";
import { type HonoEnv, registerAuthHandler } from "../../src/services/auth";
import type { PluginResult } from "../../src/services/plugins";

const createMockResponse = (body: string, status = 200, headers: Record<string, string> = {}) => {
  return new Response(body, {
    status,
    headers: new Headers(headers),
  });
};

describe("API Proxy", () => {
  describe("proxyRequest", () => {
    let fetchMock: Mock;
    const originalFetch = globalThis.fetch;

    beforeAll(() => {
      fetchMock = vi.fn();
      globalThis.fetch = fetchMock as unknown as typeof fetch;
    });

    afterAll(() => {
      globalThis.fetch = originalFetch;
    });

    beforeEach(() => {
      fetchMock.mockClear();
    });

    it("forwards request to target URL with path preserved", async () => {
      const targetBase = "https://api.example.com";
      const originalUrl = "http://localhost:3000/api/users?page=1";

      fetchMock.mockResolvedValueOnce(createMockResponse('{"users":[]}'));

      const req = new Request(originalUrl, { method: "GET" });
      await proxyRequest(req, targetBase);

      expect(fetchMock).toHaveBeenCalledTimes(1);
      const proxiedRequest = fetchMock.mock.calls[0][0] as Request;
      expect(proxiedRequest.url).toBe("https://api.example.com/api/users?page=1");
    });

    it("removes host header from forwarded request", async () => {
      const targetBase = "https://api.example.com";

      fetchMock.mockResolvedValueOnce(createMockResponse("ok"));

      const req = new Request("http://localhost:3000/api/test", {
        method: "GET",
        headers: { Host: "localhost:3000" },
      });
      await proxyRequest(req, targetBase);

      const proxiedRequest = fetchMock.mock.calls[0][0] as Request;
      expect(proxiedRequest.headers.has("host")).toBe(false);
    });

    it("sets accept-encoding to identity", async () => {
      const targetBase = "https://api.example.com";

      fetchMock.mockResolvedValueOnce(createMockResponse("ok"));

      const req = new Request("http://localhost:3000/api/test", {
        method: "GET",
        headers: { "accept-encoding": "gzip, deflate" },
      });
      await proxyRequest(req, targetBase);

      const proxiedRequest = fetchMock.mock.calls[0][0] as Request;
      expect(proxiedRequest.headers.get("accept-encoding")).toBe("identity");
    });

    it("forwards POST body correctly", async () => {
      const targetBase = "https://api.example.com";
      const body = JSON.stringify({ name: "test" });

      fetchMock.mockResolvedValueOnce(createMockResponse('{"id":1}'));

      const req = new Request("http://localhost:3000/api/users", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body,
      });
      await proxyRequest(req, targetBase);

      const proxiedRequest = fetchMock.mock.calls[0][0] as Request;
      expect(proxiedRequest.method).toBe("POST");
      expect(await proxiedRequest.text()).toBe(body);
    });

    it("preserves response status code", async () => {
      const targetBase = "https://api.example.com";

      fetchMock.mockResolvedValueOnce(createMockResponse("Not Found", 404));

      const req = new Request("http://localhost:3000/api/missing");
      const response = await proxyRequest(req, targetBase);

      expect(response.status).toBe(404);
    });

    it("removes content-encoding and content-length from response", async () => {
      const targetBase = "https://api.example.com";

      fetchMock.mockResolvedValueOnce(
        createMockResponse("ok", 200, {
          "content-encoding": "gzip",
          "content-length": "100",
        }),
      );

      const req = new Request("http://localhost:3000/api/test");
      const response = await proxyRequest(req, targetBase);

      expect(response.headers.has("content-encoding")).toBe(false);
      expect(response.headers.has("content-length")).toBe(false);
    });

    describe("cookie rewriting", () => {
      it("rewrites better-auth cookies in request when enabled", async () => {
        const targetBase = "https://api.example.com";

        fetchMock.mockResolvedValueOnce(createMockResponse("ok"));

        const req = new Request("http://localhost:3000/api/auth/session", {
          headers: {
            cookie: "better-auth.session_token=abc123; other-cookie=value",
          },
        });
        await proxyRequest(req, targetBase, true);

        const proxiedRequest = fetchMock.mock.calls[0][0] as Request;
        expect(proxiedRequest.headers.get("cookie")).toBe(
          "__Secure-better-auth.session_token=abc123; other-cookie=value",
        );
      });

      it("does not rewrite cookies when disabled", async () => {
        const targetBase = "https://api.example.com";

        fetchMock.mockResolvedValueOnce(createMockResponse("ok"));

        const req = new Request("http://localhost:3000/api/data", {
          headers: {
            cookie: "better-auth.session_token=abc123",
          },
        });
        await proxyRequest(req, targetBase, false);

        const proxiedRequest = fetchMock.mock.calls[0][0] as Request;
        expect(proxiedRequest.headers.get("cookie")).toBe("better-auth.session_token=abc123");
      });

      it("strips Secure and Domain from response cookies when rewriting", async () => {
        const targetBase = "https://api.example.com";

        const mockResponse = new Response("ok", {
          status: 200,
          headers: new Headers(),
        });

        Object.defineProperty(mockResponse.headers, "getSetCookie", {
          value: () => [
            "__Secure-better-auth.session_token=xyz; Domain=api.example.com; Secure; Path=/; HttpOnly",
          ],
        });

        fetchMock.mockResolvedValueOnce(mockResponse);

        const req = new Request("http://localhost:3000/api/auth/login");
        const response = await proxyRequest(req, targetBase, true);

        const setCookie = response.headers.get("set-cookie");
        expect(setCookie).toBeDefined();
        expect(setCookie).toContain("better-auth.session_token=xyz");
        expect(setCookie).not.toContain("__Secure-");
        expect(setCookie).not.toContain("Domain=");
        expect(setCookie).not.toContain("; Secure");
      });
    });
  });

  describe("setupApiRoutes with proxy mode", () => {
    let fetchMock: Mock;
    const originalFetch = globalThis.fetch;

    beforeAll(() => {
      fetchMock = vi.fn();
      globalThis.fetch = fetchMock as unknown as typeof fetch;
    });

    afterAll(() => {
      globalThis.fetch = originalFetch;
    });

    beforeEach(() => {
      fetchMock.mockClear();
    });

    it("proxies /api/* routes when proxy is configured", async () => {
      const app = new Hono();

      const config = {
        hostUrl: "http://localhost:3000",
        api: {
          name: "api",
          url: "http://localhost:3001",
          source: "local" as const,
          proxy: "https://production-api.example.com",
        },
        ui: {
          name: "ui",
          url: "http://localhost:3003",
          source: "local" as const,
        },
      };

      app.all("/api/*", async (c: Context) => {
        const response = await proxyRequest(c.req.raw, config.api.proxy!, true);
        return response;
      });

      fetchMock.mockResolvedValueOnce(createMockResponse('{"status":"ok"}'));

      await app.fetch(new Request("http://localhost:3000/api/health"));
      expect(fetchMock).toHaveBeenCalled();

      const proxiedRequest = fetchMock.mock.calls[0][0] as Request;
      expect(proxiedRequest.url).toBe("https://production-api.example.com/api/health");
    });

    it("handles RPC requests through proxy", async () => {
      const app = new Hono();

      app.all("/api/*", async (c: Context) => {
        const response = await proxyRequest(c.req.raw, "https://production-api.example.com", true);
        return response;
      });

      const rpcBody = JSON.stringify({ method: "getValue", params: { key: "test" } });
      fetchMock.mockResolvedValueOnce(createMockResponse('{"result":"value"}'));

      await app.fetch(
        new Request("http://localhost:3000/api/rpc/getValue", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: rpcBody,
        }),
      );

      expect(fetchMock).toHaveBeenCalled();
      const proxiedRequest = fetchMock.mock.calls[0][0] as Request;
      expect(proxiedRequest.url).toBe("https://production-api.example.com/api/rpc/getValue");
      expect(proxiedRequest.method).toBe("POST");
    });
  });

  describe("setupApiRoutes auth route precedence", () => {
    it("handles /api/auth before the generic API router", async () => {
      const app = new Hono<HonoEnv>();
      const authHandler = vi.fn(async () => new Response("auth ok", { status: 200 }));

      const config = {
        hostUrl: "http://localhost:3000",
        api: {
          name: "api",
          url: "http://localhost:3001",
          source: "local" as const,
        },
        ui: {
          name: "ui",
          url: "http://localhost:3003",
          source: "local" as const,
        },
      } as any;

      const plugins: PluginResult = {
        runtime: null,
        auth: {
          key: "auth",
          name: "auth",
          createClient: () => ({}),
          router: {},
          metadata: { remoteUrl: "local" },
          initialized: {
            context: {
              handler: authHandler,
              auth: { api: { getSession: vi.fn() } },
              db: {} as any,
            },
          },
        } as any,
        api: null,
        plugins: {},
        authClient: null,
        status: {
          available: false,
          pluginName: null,
          error: null,
          errorDetails: null,
          loadedPlugins: [],
        },
      };

      registerAuthHandler(app, plugins);

      setupApiRoutes(app, config, plugins, async (_c, next) => next(), {
        status: "ready",
        startTime: Date.now(),
        milestones: [],
        error: null,
        ssrEnabled: false,
      });

      const response = await app.fetch(new Request("http://localhost:3000/api/auth/session"));

      expect(response.status).toBe(200);
      expect(await response.text()).toBe("auth ok");
      expect(authHandler).toHaveBeenCalledTimes(1);
    });
  });

  describe("RuntimeConfig proxy configuration", () => {
    it("correctly builds RuntimeConfig with proxy URL", () => {
      const bosConfig = {
        account: "test.near",
        app: {
          host: { development: "http://localhost:3000", production: "https://prod.example.com" },
          ui: {
            name: "ui",
            development: "http://localhost:3003",
            production: "https://ui.example.com",
          },
          api: {
            name: "api",
            development: "http://localhost:3001",
            production: "https://api.example.com",
            proxy: "https://staging-api.example.com",
          },
        },
      };

      const runtimeConfig = {
        env: "development" as const,
        account: bosConfig.account,
        hostUrl: "http://localhost:3000",
        ui: {
          name: bosConfig.app.ui.name,
          url: bosConfig.app.ui.development,
          source: "local" as const,
        },
        api: {
          name: bosConfig.app.api.name,
          url: bosConfig.app.api.development,
          source: "local" as const,
          proxy: bosConfig.app.api.proxy,
        },
      };

      expect(runtimeConfig.api.proxy).toBe("https://staging-api.example.com");
    });

    it("validates proxy URL is a valid URL", () => {
      const isValidUrl = (url: string): boolean => {
        try {
          new URL(url);
          return true;
        } catch {
          return false;
        }
      };

      expect(isValidUrl("https://api.example.com")).toBe(true);
      expect(isValidUrl("http://localhost:3001")).toBe(true);
      expect(isValidUrl("true")).toBe(false);
      expect(isValidUrl("invalid")).toBe(false);
      expect(isValidUrl("")).toBe(false);
    });
  });
});
