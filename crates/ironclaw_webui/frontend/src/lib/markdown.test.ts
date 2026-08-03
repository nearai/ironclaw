// @vitest-environment happy-dom
import assert from "node:assert/strict";
import { afterEach, test, vi } from "vitest";

// `renderMarkdown` is loaded only after a response stabilizes, so the
// security-relevant invariant — every parsed payload passes through
// DOMPurify.sanitize — must be pinned so deferred loading cannot silently drop
// sanitization.
//
// Stub the npm imports per test so each case is isolated and the real browser
// DOMPurify implementation is never needed under Vitest's node environment.
type MarkdownMocks = {
  parse: (...args: Array<any>) => unknown;
  sanitize: (raw: string) => string;
  addHook?: (...args: Array<any>) => void;
};

async function loadRenderMarkdown({
  parse,
  sanitize,
  addHook = () => {},
}: MarkdownMocks) {
  vi.resetModules();
  vi.doMock("marked", () => ({
    marked: { parse },
  }));
  vi.doMock("dompurify", () => ({
    default: () => ({ addHook, sanitize }),
  }));

  const mod = await import("./markdown");
  return mod.renderMarkdown;
}

afterEach(() => {
  vi.doUnmock("marked");
  vi.doUnmock("dompurify");
});

test("renderMarkdown routes parsed HTML through DOMPurify.sanitize, stripping handlers", async () => {
  const calls = { parse: [], sanitize: [] };
  const renderMarkdown = await loadRenderMarkdown({
    // Pass the dangerous markup straight through so the only thing
    // that can strip it is the sanitize step.
    parse: (content, opts) => {
      calls.parse.push({ content, opts });
      return `<p>${content}</p>`;
    },
    sanitize: (raw) => {
      calls.sanitize.push(raw);
      return raw.replace(/ onerror="[^"]*"/g, "");
    },
  });

  const out = renderMarkdown('<img src=x onerror="alert(1)">');

  assert.equal(calls.parse.length, 1, "content is parsed once");
  assert.equal(calls.parse[0].opts.gfm, true, "marked is called with gfm: true");
  assert.equal(calls.parse[0].opts.breaks, true, "marked is called with breaks: true");
  assert.equal(calls.parse[0].opts.async, false, "marked stays synchronous");
  assert.equal(calls.sanitize.length, 1, "parsed output passes through sanitize exactly once");
  assert.equal(
    calls.sanitize[0],
    '<p><img src=x onerror="alert(1)"></p>',
    "sanitize receives the PARSED HTML, not the raw input — order is parse-then-sanitize",
  );
  assert.ok(!out.includes("onerror"), "the dangerous handler is stripped by the sanitize pass");
  assert.equal(out, "<p><img src=x></p>", "renderMarkdown returns sanitize's output, never raw markup");
});

test("renderMarkdown installs workspace and external-link hooks once", async () => {
  const calls = { hooks: [] };
  const renderMarkdown = await loadRenderMarkdown({
    parse: (content) => `<p>${content}</p>`,
    sanitize: (raw) => raw,
    addHook: (name, hook) => {
      calls.hooks.push({ name, hook });
    },
  });

  renderMarkdown("[example](https://example.com)");
  renderMarkdown("[again](https://example.com)");

  assert.equal(calls.hooks.length, 2, "each DOMPurify link hook is registered only once");
  const targetHook = calls.hooks.find(
    ({ name }) => name === "afterSanitizeAttributes",
  );
  assert.ok(targetHook);

  const attrs = new Map([["href", "https://example.com"]]);
  const node = {
    tagName: "A",
    getAttribute: (name) => attrs.get(name) || null,
    setAttribute: (name, value) => attrs.set(name, value),
    removeAttribute: (name) => attrs.delete(name),
  };
  targetHook.hook(node);

  assert.equal(attrs.get("target"), "_blank");
  assert.equal(attrs.get("rel"), "noopener noreferrer");
});

test("renderMarkdown isolates sanitizers by workspace-link capability", async () => {
  vi.resetModules();
  const created = [];
  vi.doMock("marked", () => ({
    marked: {
      parse: (content) => `<p>${content}</p>`,
    },
  }));
  vi.doMock("dompurify", () => ({
    default: () => {
      const hooks = [];
      created.push(hooks);
      return {
        addHook: (name) => hooks.push(name),
        sanitize: (raw) => raw,
      };
    },
  }));

  const { renderMarkdown } = await import("./markdown");
  renderMarkdown("default");
  renderMarkdown("preview", { workspaceFileLinks: true });
  renderMarkdown("default again");

  assert.equal(created.length, 2, "each capability mode reuses its own sanitizer");
  assert.deepEqual(created[0], [
    "uponSanitizeAttribute",
    "afterSanitizeAttributes",
  ]);
  assert.deepEqual(created[1], [
    "uponSanitizeAttribute",
    "afterSanitizeAttributes",
  ]);
});

test("renderMarkdown returns an empty string for falsy content", async () => {
  const renderMarkdown = await loadRenderMarkdown({
    parse: () => {
      throw new Error("parse should not run for falsy content");
    },
    sanitize: () => {
      throw new Error("sanitize should not run for falsy content");
    },
  });
  assert.equal(renderMarkdown(""), "");
  assert.equal(renderMarkdown(null), "");
  assert.equal(renderMarkdown(undefined), "");
});

test("workspace sanitizer hooks canonicalize links and reject forged metadata", async () => {
  vi.resetModules();
  const created = [];
  vi.doMock("marked", () => ({
    marked: { parse: (content) => content },
  }));
  vi.doMock("dompurify", () => ({
    default: () => {
      const hooks = new Map();
      created.push(hooks);
      return {
        addHook: (name, hook) => hooks.set(name, hook),
        sanitize: (raw) => raw,
      };
    },
  }));

  const { renderMarkdown } = await import("./markdown");
  renderMarkdown("preview", { workspaceFileLinks: true });
  renderMarkdown("default");

  const previewHooks = created[0];
  const defaultHooks = created[1];
  const sanitizeAnchor = (hooks, href, forgedPath = undefined) => {
    const attrs = new Map([["href", href]]);
    if (forgedPath) attrs.set("data-workspace-path", forgedPath);
    const node = {
      tagName: "A",
      getAttribute: (name) => attrs.get(name) || null,
      setAttribute: (name, value) => attrs.set(name, value),
      removeAttribute: (name) => attrs.delete(name),
    };
    const hrefData = { attrName: "href", attrValue: href };
    hooks.get("uponSanitizeAttribute")?.(node, hrefData);
    attrs.set("href", hrefData.attrValue);
    hooks.get("afterSanitizeAttributes")?.(node);
    return attrs;
  };

  const workspace = sanitizeAnchor(
    previewHooks,
    "sandbox:/workspace/%E6%8A%A5%E5%91%8A%20final.md",
    "/workspace/forged.md",
  );
  assert.equal(
    workspace.get("href"),
    "/workspace/%E6%8A%A5%E5%91%8A%20final.md",
  );
  assert.equal(workspace.get("data-workspace-path"), "/workspace/报告 final.md");
  assert.equal(workspace.get("target"), "_blank");
  assert.equal(workspace.get("rel"), "noopener noreferrer");

  const external = sanitizeAnchor(
    previewHooks,
    "https://example.com",
    "/workspace/forged.md",
  );
  assert.equal(external.has("data-workspace-path"), false);

  const defaultWorkspace = sanitizeAnchor(
    defaultHooks,
    "/workspace/plain.txt",
    "/workspace/forged.md",
  );
  assert.equal(defaultWorkspace.has("data-workspace-path"), false);
});
