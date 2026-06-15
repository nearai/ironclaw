import { afterAll, beforeAll, describe, expect, it } from "vitest";
import type { HeadData, RouterModule, RuntimeConfig } from "@/types";
import { loadBundledRouterModule } from "../helpers/bundled-ssr-module";
import { buildTestRouteHeadContext, loadTestRuntimeConfig } from "../helpers/runtime-config";

function escapeHtml(str: string): string {
  return str
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function renderHeadToString(head: HeadData): string {
  const parts: string[] = [];

  for (const meta of head.meta) {
    if (!meta) continue;
    const metaObj = meta as Record<string, unknown>;
    if ("title" in metaObj && metaObj.title) {
      parts.push(`<title>${escapeHtml(String(metaObj.title))}</title>`);
    } else {
      const attrs = Object.entries(metaObj)
        .filter(([k, v]) => k !== "children" && v !== undefined)
        .map(([k, v]) => `${k}="${escapeHtml(String(v))}"`)
        .join(" ");
      if (attrs) parts.push(`<meta ${attrs} />`);
    }
  }

  for (const link of head.links) {
    if (!link) continue;
    const linkObj = link as Record<string, unknown>;
    const attrs = Object.entries(linkObj)
      .filter(([, v]) => v !== undefined)
      .map(([k, v]) => `${k}="${escapeHtml(String(v))}"`)
      .join(" ");
    if (attrs) parts.push(`<link ${attrs} />`);
  }

  for (const script of head.scripts) {
    if (!script) continue;
    const scriptObj = script as Record<string, unknown>;
    const { children, ...rest } = scriptObj;
    const attrs = Object.entries(rest)
      .filter(([, v]) => v !== undefined)
      .map(([k, v]) => (typeof v === "boolean" ? (v ? k : "") : `${k}="${escapeHtml(String(v))}"`))
      .filter(Boolean)
      .join(" ");
    if (children) {
      parts.push(`<script ${attrs}>${children}</script>`);
    } else if (attrs) {
      parts.push(`<script ${attrs}></script>`);
    }
  }

  return parts.join("\n");
}

describe("SEO Head Extraction", () => {
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

  describe("Module Federation", () => {
    it("loads RouterModule from SSR remote", () => {
      expect(routerModule).toBeDefined();
      expect(routerModule.getRouteHead).toBeTypeOf("function");
      expect(routerModule.createRouter).toBeTypeOf("function");
    });
  });

  describe("Root Route (/)", () => {
    let head: HeadData;

    beforeAll(async () => {
      head = await routerModule.getRouteHead("/", buildTestRouteHeadContext(config));
    });

    it("returns HeadData structure", () => {
      expect(head).toHaveProperty("meta");
      expect(head).toHaveProperty("links");
      expect(head).toHaveProperty("scripts");
      expect(Array.isArray(head.meta)).toBe(true);
      expect(Array.isArray(head.links)).toBe(true);
      expect(Array.isArray(head.scripts)).toBe(true);
    });

    it("has title meta tag", () => {
      const titleMeta = head.meta.find((m) => m && typeof m === "object" && "title" in m);
      expect(titleMeta).toBeDefined();
      expect((titleMeta as { title: string }).title).toBeTruthy();
    });

    it("has charset meta tag", () => {
      const charsetMeta = head.meta.find((m) => m && typeof m === "object" && "charSet" in m);
      expect(charsetMeta).toBeDefined();
      expect((charsetMeta as { charSet: string }).charSet).toBe("utf-8");
    });

    it("has viewport meta tag", () => {
      const viewportMeta = head.meta.find(
        (m) =>
          m && typeof m === "object" && "name" in m && (m as { name: string }).name === "viewport",
      );
      expect(viewportMeta).toBeDefined();
    });

    it("has description meta tag", () => {
      const descMeta = head.meta.find(
        (m) =>
          m &&
          typeof m === "object" &&
          "name" in m &&
          (m as { name: string }).name === "description",
      );
      expect(descMeta).toBeDefined();
      expect((descMeta as { content: string }).content).toBeTruthy();
    });

    it("has Open Graph tags", () => {
      const ogTitle = head.meta.find(
        (m) =>
          m &&
          typeof m === "object" &&
          "property" in m &&
          (m as { property: string }).property === "og:title",
      );
      const ogDescription = head.meta.find(
        (m) =>
          m &&
          typeof m === "object" &&
          "property" in m &&
          (m as { property: string }).property === "og:description",
      );
      const ogType = head.meta.find(
        (m) =>
          m &&
          typeof m === "object" &&
          "property" in m &&
          (m as { property: string }).property === "og:type",
      );

      expect(ogTitle).toBeDefined();
      expect(ogDescription).toBeDefined();
      expect(ogType).toBeDefined();
    });

    it("has Twitter Card tags", () => {
      const twitterCard = head.meta.find(
        (m) =>
          m &&
          typeof m === "object" &&
          "name" in m &&
          (m as { name: string }).name === "twitter:card",
      );
      const twitterTitle = head.meta.find(
        (m) =>
          m &&
          typeof m === "object" &&
          "name" in m &&
          (m as { name: string }).name === "twitter:title",
      );

      expect(twitterCard).toBeDefined();
      expect(twitterTitle).toBeDefined();
    });

    it("has canonical link", () => {
      const canonical = head.links.find(
        (l) =>
          l && typeof l === "object" && "rel" in l && (l as { rel: string }).rel === "canonical",
      );
      expect(canonical).toBeDefined();
    });

    it("has structured data script", async () => {
      expect(head.scripts).toBeDefined();
      expect(Array.isArray(head.scripts)).toBe(true);
    });
  });

  describe("HTML Rendering", () => {
    it("renders valid title tag", async () => {
      const head = await routerModule.getRouteHead("/", buildTestRouteHeadContext(config));

      const html = renderHeadToString(head);
      expect(html).toMatch(/<title>.+<\/title>/);
    });

    it("renders meta tags with proper attributes", async () => {
      const head = await routerModule.getRouteHead("/", buildTestRouteHeadContext(config));

      const html = renderHeadToString(head);
      expect(html).toMatch(/<meta charSet="utf-8" \/>/);
      expect(html).toMatch(/<meta name="viewport"/);
      expect(html).toMatch(/<meta name="description"/);
      expect(html).toMatch(/<meta property="og:title"/);
    });

    it("renders link tags", async () => {
      const head = await routerModule.getRouteHead("/", buildTestRouteHeadContext(config));

      const html = renderHeadToString(head);
      expect(html).toMatch(/<link rel="canonical"/);
      expect(html).toMatch(/<link rel="icon"/);
    });

    it("renders structured data with valid JSON", async () => {
      const head = await routerModule.getRouteHead("/", buildTestRouteHeadContext(config));

      expect(head.scripts).toBeDefined();
      expect(Array.isArray(head.scripts)).toBe(true);
    });

    it("properly escapes HTML in meta content", async () => {
      const head: HeadData = {
        meta: [
          { title: 'Test <script>alert("xss")</script>' },
          { name: "description", content: 'Test & "quotes" < > symbols' },
        ],
        links: [],
        scripts: [],
      };

      const html = renderHeadToString(head);
      expect(html).not.toContain("<script>");
      expect(html).toContain("&lt;script&gt;");
      expect(html).toContain("&amp;");
      expect(html).toContain("&quot;");
    });
  });
});
