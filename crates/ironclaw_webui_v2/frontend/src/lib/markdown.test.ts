import assert from "node:assert/strict";
import { afterEach, test, vi } from "vitest";

// `renderMarkdown` is memo-cached by the renderer (markdown-renderer.tsx), so
// the security-relevant invariant — every parsed payload passes through
// DOMPurify.sanitize — must be pinned so a future change to the memo deps or to
// this function cannot silently drop sanitization.
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
    default: { addHook, sanitize },
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

test("renderMarkdown installs the external-link hardening hook once", async () => {
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

  assert.equal(calls.hooks.length, 1, "DOMPurify hook is registered only once");
  assert.equal(calls.hooks[0].name, "afterSanitizeAttributes");

  const attrs = new Map([["href", "https://example.com"]]);
  const node = {
    tagName: "A",
    getAttribute: (name) => attrs.get(name) || null,
    setAttribute: (name, value) => attrs.set(name, value),
  };
  calls.hooks[0].hook(node);

  assert.equal(attrs.get("target"), "_blank");
  assert.equal(attrs.get("rel"), "noopener noreferrer");
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
