import { existsSync } from "node:fs";
import path from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { runServer } from "../../src/program";
import type { RuntimeConfig } from "../../src/services/config";
import { startJsonProxyTarget } from "../helpers/json-proxy-target";
import { getAvailablePort } from "../helpers/ports";
import { startStaticDistServer } from "../helpers/static-dist-server";
import { loadHostTestEnv } from "../helpers/test-env";

const workspaceRoot = path.resolve(import.meta.dirname, "../../..");
const uiPublicDir = path.join(workspaceRoot, "ui", "public");
const uiDistDir = path.join(workspaceRoot, "ui", "dist");

loadHostTestEnv(workspaceRoot);

function buildUiRemoteConfig(
  uiAssetsUrl: string,
  apiProxyUrl: string,
  hostUrl: string,
): RuntimeConfig {
  return {
    env: "development",
    account: "dev.everything.near",
    title: "everything.dev",
    repository: "https://github.com/nearbuilders/everything-dev",
    host: {
      name: "host",
      url: hostUrl,
      entry: `${hostUrl}/mf-manifest.json`,
      source: "remote",
    },
    ui: {
      name: "ui",
      url: uiAssetsUrl,
      entry: `${uiAssetsUrl}/mf-manifest.json`,
      source: "remote",
    },
    api: {
      name: "api",
      url: apiProxyUrl,
      entry: `${apiProxyUrl}/mf-manifest.json`,
      source: "remote",
      proxy: apiProxyUrl,
    },
  } as RuntimeConfig;
}

describe("UI public assets proxied through host (Cloudflare Error 1000 regression)", () => {
  let uiServer: Awaited<ReturnType<typeof startStaticDistServer>>;
  let apiProxy: Awaited<ReturnType<typeof startJsonProxyTarget>>;
  let hostHandle: ReturnType<typeof runServer>;
  let baseUrl: string;
  const envSnapshot = { ...process.env };

  beforeAll(async () => {
    if (!existsSync(uiPublicDir)) {
      throw new Error(`ui/public/ not found at ${uiPublicDir} — run UI build first`);
    }

    uiServer = await startStaticDistServer(uiDistDir);
    apiProxy = await startJsonProxyTarget();

    const hostPort = await getAvailablePort();
    baseUrl = `http://127.0.0.1:${hostPort}`;
    process.env.NODE_ENV = "development";
    process.env.HOST = "127.0.0.1";
    process.env.PORT = String(hostPort);
    process.env.CSP_STRICT = "false";

    const config = buildUiRemoteConfig(uiServer.baseUrl, apiProxy.baseUrl, baseUrl);
    hostHandle = runServer({ config });
    await hostHandle.ready;
  }, 30000);

  afterAll(async () => {
    await hostHandle?.shutdown();
    await uiServer?.stop();
    await apiProxy?.stop();
    process.env = { ...envSnapshot };
  });

  describe("UI assets are proxied through the host with 200", () => {
    it("proxies /favicon.ico with 200", async () => {
      const response = await fetch(`${baseUrl}/favicon.ico`);

      expect(response.status).toBe(200);
      const buf = await response.arrayBuffer();
      expect(buf.byteLength).toBeGreaterThan(0);
    });

    it("proxies /icon.svg with 200", async () => {
      const response = await fetch(`${baseUrl}/icon.svg`);

      expect(response.status).toBe(200);
      const buf = await response.arrayBuffer();
      expect(buf.byteLength).toBeGreaterThan(0);
    });

    it("proxies /skill.md with 200", async () => {
      const response = await fetch(`${baseUrl}/skill.md`);

      expect(response.status).toBe(200);
      const text = await response.text();
      expect(text.length).toBeGreaterThan(0);
    });

    it("proxies /robots.txt with 200", async () => {
      const response = await fetch(`${baseUrl}/robots.txt`);

      expect(response.status).toBe(200);
      const text = await response.text();
      expect(text.length).toBeGreaterThan(0);
    });

    it("proxies /manifest.json with 200", async () => {
      const response = await fetch(`${baseUrl}/manifest.json`);

      expect(response.status).toBe(200);
      const json = (await response.json()) as Record<string, unknown>;
      expect(json).toHaveProperty("name");
    });

    it("proxies a hashed static asset path", async () => {
      const response = await fetch(`${baseUrl}/static/css/style.css`);

      expect(response.status).toBe(200);
    });
  });

  describe("HTML pages use the UI asset origin for executable assets", () => {
    it("/ renders client shell with UI boot assets on the UI origin", async () => {
      const response = await fetch(`${baseUrl}/`);

      expect(response.status).toBe(200);
      const html = await response.text();
      expect(html).toContain('href="/favicon.ico"');
      expect(html).toContain(`${uiServer.baseUrl}/remoteEntry.js`);
      expect(html).toContain(`${uiServer.baseUrl}/static/css/style.css`);
    });
  });

  describe("non-asset paths are not proxied", () => {
    it("/ (root) renders client shell", async () => {
      const response = await fetch(`${baseUrl}/`);

      expect(response.status).toBe(200);
      const html = await response.text();
      expect(html).toContain("window.__RUNTIME_CONFIG__");
      expect(html).toContain("remoteEntry.js");
    });

    it("/health is handled by host directly", async () => {
      const response = await fetch(`${baseUrl}/health`);

      expect(response.status).toBe(200);
      expect(await response.text()).toBe("OK");
    });

    it("/api/ping is routed to API proxy", async () => {
      const response = await fetch(`${baseUrl}/api/ping`);
      const json = (await response.json()) as Record<string, unknown>;

      expect(response.status).toBe(200);
      expect(json).toMatchObject({ status: "ok" });
    });

    it("paths without file extensions render client shell", async () => {
      const response = await fetch(`${baseUrl}/nonexistent-page`);

      expect(response.status).toBe(200);
      const html = await response.text();
      expect(html).toContain("window.__RUNTIME_CONFIG__");
    });
  });

  describe("missing UI assets return 404 from proxied target", () => {
    it("returns 404 for a nonexistent asset", async () => {
      const response = await fetch(`${baseUrl}/nonexistent-file.css`);

      expect(response.status).toBe(404);
    });
  });
});
