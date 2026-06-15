import { afterAll, beforeAll, describe, expect, it } from "vitest";
import type { RouterModule, RuntimeConfig } from "@/types";
import { createTestApiClient } from "../helpers/api-client";
import { loadBundledRouterModule } from "../helpers/bundled-ssr-module";
import {
  buildTestRenderOptions,
  buildTestRouteHeadContext,
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

describe("SSR Stream Lifecycle", () => {
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

  describe("Stream Completion", () => {
    it("completes stream for /skill route without timeout", async () => {
      const startTime = Date.now();

      const head = await routerModule.getRouteHead("/skill", buildTestRouteHeadContext(config));

      const elapsed = Date.now() - startTime;

      expect(head).toBeDefined();
      expect(head.meta).toBeDefined();
      expect(elapsed).toBeLessThan(5000);
    });
  });

  describe("SSR Configuration", () => {
    it("renders layout route metadata", async () => {
      const head = await routerModule.getRouteHead("/skill", buildTestRouteHeadContext(config));

      const titleMeta = head.meta.find((m) => m && typeof m === "object" && "title" in m);
      expect(titleMeta).toBeDefined();
    });
  });

  describe("SSR Routes", () => {
    const STREAM_TIMEOUT = 5000;

    it("renders /skill with full SSR", { timeout: 6000 }, async () => {
      const request = new Request("http://localhost/skill");
      const startTime = Date.now();

      const result = await routerModule.renderToStream(
        request,
        buildTestRenderOptions(config, mockApiClient, mockAuthClient),
      );

      const html = await consumeStream(result.stream);
      const elapsed = Date.now() - startTime;

      expect(elapsed).toBeLessThan(STREAM_TIMEOUT);
      expect(result.statusCode).toBe(200);
      expect(html).toContain("<!DOCTYPE html>");
      expect(html).toContain("</html>");
      expect(html).toContain("Setup Skill");
    });
  });

  describe("Full Stream Rendering", () => {
    const STREAM_TIMEOUT = 5000;

    it("completes full stream render for /skill", { timeout: 6000 }, async () => {
      const request = new Request("http://localhost/skill");
      const startTime = Date.now();

      const result = await routerModule.renderToStream(
        request,
        buildTestRenderOptions(config, mockApiClient, mockAuthClient),
      );

      const html = await consumeStream(result.stream);
      const elapsed = Date.now() - startTime;

      expect(elapsed).toBeLessThan(STREAM_TIMEOUT);
      expect(result.statusCode).toBe(200);
      expect(html).toContain("<!DOCTYPE html>");
      expect(html).toContain("</html>");
      expect(html).toContain("Setup Skill");
    });
  });
});
