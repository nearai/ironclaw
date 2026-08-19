// @vitest-environment jsdom
import assert from "node:assert/strict";
import { test, vi } from "vitest";
import { renderMarkdown } from "./markdown";

test("renderMarkdown parses markdown and sanitizes dangerous attributes", () => {
  const out = renderMarkdown(
    '**safe** <img src="x" onerror="globalThis.compromised = true">',
  );

  assert.match(out, /<strong>safe<\/strong>/);
  assert.match(out, /<img src="x">/);
  assert.doesNotMatch(out, /onerror|compromised/);
});

test("renderMarkdown hardens external links", () => {
  const out = renderMarkdown("[example](https://example.com)");

  assert.match(out, /href="https:\/\/example\.com"/);
  assert.match(out, /target="_blank"/);
  assert.match(out, /rel="noopener noreferrer"/);
});

test("renderMarkdown returns an empty string for falsy content", () => {
  assert.equal(renderMarkdown(""), "");
  assert.equal(renderMarkdown(null), "");
  assert.equal(renderMarkdown(undefined), "");
});

test("renderMarkdown renders gemoji shortcodes outside code", () => {
  const out = renderMarkdown(
    "Hello :wave: :smile: :sunglasses: :+1:\n\n`:wave:`\n\n```text\n:wave:\n```",
  );

  assert.match(out, /Hello 👋 😄 😎 👍/);
  assert.match(out, /<code>:wave:<\/code>/);
  assert.match(out, /<pre><code class="language-text">:wave:/);
});

test("renderMarkdown fails closed when DOMPurify is unsupported", async () => {
  vi.resetModules();
  vi.doMock("dompurify", () => ({
    default: () => ({ isSupported: false }),
  }));

  try {
    const { renderMarkdown: renderWithoutSanitizer } = await import(
      "./markdown"
    );
    assert.throws(
      () => renderWithoutSanitizer('<img src="x" onerror="compromised()">'),
      /Markdown sanitization is unavailable/,
    );
  } finally {
    vi.doUnmock("dompurify");
  }
});
