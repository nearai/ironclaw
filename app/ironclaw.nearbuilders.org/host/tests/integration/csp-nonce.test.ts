import { afterAll, beforeAll, describe, expect, it } from "vitest";
import type { RenderOptionsWithApi, RouterModule, RuntimeConfig } from "@/types";
import type { ApiClient } from "../../../ui/src/lib/api";
import { createTestApiClient } from "../helpers/api-client";
import { loadBundledRouterModule } from "../helpers/bundled-ssr-module";
import {
  buildTestClientRuntimeConfig,
  createMockAuthClient,
  loadTestRuntimeConfig,
} from "../helpers/runtime-config";

async function consumeStream(stream: ReadableStream): Promise<string> {
  const reader = stream.getReader();
  const decoder = new TextDecoder();
  let html = "";
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    html += decoder.decode(value, { stream: true });
  }
  html += decoder.decode();
  return html;
}

const mockApiClient = createTestApiClient({});
const mockAuthClient = createMockAuthClient();

describe("CSP Nonce Regression Tests", () => {
  let routerModule: RouterModule;
  let config: RuntimeConfig;
  let cleanup: () => Promise<void>;

  beforeAll(async () => {
    config = await loadTestRuntimeConfig();
    const bundled = await loadBundledRouterModule();
    routerModule = bundled.routerModule;
    cleanup = bundled.cleanup;
  });

  afterAll(async () => {
    await cleanup();
  });

  describe("SSR renders script tags with nonce when cspNonce is provided", () => {
    it("includes nonce attribute on <script> tags in SSR output", { timeout: 6000 }, async () => {
      const testNonce = "regression-test-nonce-abc123";
      const request = new Request("http://localhost/");

      const renderOptions: RenderOptionsWithApi<ApiClient> = {
        runtimeConfig: buildTestClientRuntimeConfig(config),
        apiClient: mockApiClient,
        session: null,
        authClient: mockAuthClient,
        cspNonce: testNonce,
      };

      const result = await routerModule.renderToStream(request, renderOptions);
      const html = await consumeStream(result.stream);

      expect(result.statusCode).toBe(200);

      const nonceScripts = html.match(/<script[^>]*nonce=["'][^"']*["'][^>]*>/g) ?? [];
      expect(
        nonceScripts.length,
        `Expected at least one <script> tag with nonce="${testNonce}", but found none. HTML snippet: ${html.slice(0, 500)}`,
      ).toBeGreaterThanOrEqual(1);

      for (const tag of nonceScripts) {
        expect(tag).toContain(`nonce="${testNonce}"`);
      }
    });

    it("includes nonce attribute on <style> tags in SSR output", { timeout: 6000 }, async () => {
      const testNonce = "regression-test-nonce-style-456";
      const request = new Request("http://localhost/");

      const renderOptions: RenderOptionsWithApi<ApiClient> = {
        runtimeConfig: buildTestClientRuntimeConfig(config),
        apiClient: mockApiClient,
        session: null,
        authClient: mockAuthClient,
        cspNonce: testNonce,
      };

      const result = await routerModule.renderToStream(request, renderOptions);
      const html = await consumeStream(result.stream);

      expect(result.statusCode).toBe(200);

      const nonceStyles = html.match(/<style[^>]*nonce=["'][^"']*["'][^>]*>/g) ?? [];
      expect(
        nonceStyles.length,
        `Expected at least one <style> tag with nonce="${testNonce}", but found none.`,
      ).toBeGreaterThanOrEqual(1);

      for (const tag of nonceStyles) {
        expect(tag).toContain(`nonce="${testNonce}"`);
      }
    });

    it("bootstraps the nonce on window for hydration", { timeout: 6000 }, async () => {
      const testNonce = "regression-test-runtime-nonce-789";
      const request = new Request("http://localhost/");

      const renderOptions: RenderOptionsWithApi<ApiClient> = {
        runtimeConfig: buildTestClientRuntimeConfig(config),
        apiClient: mockApiClient,
        session: null,
        authClient: mockAuthClient,
        cspNonce: testNonce,
      };

      const result = await routerModule.renderToStream(request, renderOptions);
      const html = await consumeStream(result.stream);

      expect(result.statusCode).toBe(200);
      expect(html).toContain(`window.__CSP_NONCE__=${JSON.stringify(testNonce)}`);
      expect(html).not.toContain('"cspNonce"');
    });

    it("does not include nonce attributes when cspNonce is omitted", {
      timeout: 6000,
    }, async () => {
      const request = new Request("http://localhost/");

      const renderOptions: RenderOptionsWithApi<ApiClient> = {
        runtimeConfig: buildTestClientRuntimeConfig(config),
        apiClient: mockApiClient,
        session: null,
        authClient: mockAuthClient,
      };

      const result = await routerModule.renderToStream(request, renderOptions);
      const html = await consumeStream(result.stream);

      expect(result.statusCode).toBe(200);

      const nonceScriptTags = html.match(/<script[^>]*nonce=["'][^"']*["'][^>]*>/g) ?? [];
      const nonceStyleTags = html.match(/<style[^>]*nonce=["'][^"']*["'][^>]*>/g) ?? [];

      expect(
        nonceScriptTags.length,
        "Expected no <script> tags with nonce when cspNonce is omitted",
      ).toBe(0);
      expect(
        nonceStyleTags.length,
        "Expected no <style> tags with nonce when cspNonce is omitted",
      ).toBe(0);
    });
  });

  describe("RenderOptions type includes cspNonce without cast", () => {
    it("accepts cspNonce as a typed property on RenderOptionsWithApi", () => {
      const options: RenderOptionsWithApi<ApiClient> = {
        runtimeConfig: {
          account: "test.near",
          env: "development",
          networkId: "mainnet",
          assetsUrl: "/assets",
          apiBase: "/api",
          rpcBase: "/rpc",
        },
        apiClient: mockApiClient,
        session: null,
        authClient: mockAuthClient,
        cspNonce: "typed-nonce-without-cast",
      };

      expect(options.cspNonce).toBe("typed-nonce-without-cast");
    });

    it("accepts RenderOptionsWithApi without cspNonce (optional)", () => {
      const options: RenderOptionsWithApi<ApiClient> = {
        runtimeConfig: {
          account: "test.near",
          env: "development",
          networkId: "mainnet",
          assetsUrl: "/assets",
          apiBase: "/api",
          rpcBase: "/rpc",
        },
        apiClient: mockApiClient,
        session: null,
        authClient: mockAuthClient,
      };

      expect(options.cspNonce).toBeUndefined();
    });
  });
});
