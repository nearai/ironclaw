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

test("renderMarkdown keeps a standalone numeric sentence visible instead of an empty ordered list", () => {
  // `19.` alone is, to CommonMark, an ordered list starting at 19 with one
  // empty item — invisible in the chat. A model's short answer must render.
  const alone = renderMarkdown("19.");
  assert.match(alone, />19\.</);
  assert.doesNotMatch(alone, /<ol/);

  const afterText = renderMarkdown("The answer:\n19.");
  assert.match(afterText, /19\./);
  assert.doesNotMatch(afterText, /<ol/);

  const withSpace = renderMarkdown("42. ");
  assert.match(withSpace, />42\.</);
  assert.doesNotMatch(withSpace, /<ol/);

  // Real lists and fenced code are untouched.
  assert.match(renderMarkdown("1. First\n2. Second"), /<ol>[\s\S]*<li>First/);
  assert.match(
    renderMarkdown("```text\n19.\n```"),
    /<pre><code class="language-text">19\.\n?<\/code>/,
  );
});

test("renderMarkdown closes a fence only on its own delimiter at its own width", () => {
  // A ``` block may contain a ~~~ line; it does not close the block, so a
  // bare number inside stays literal code, never an escaped `19\.`.
  const other = renderMarkdown("```text\n~~~\n19.\n```");
  assert.match(other, /<pre><code class="language-text">~~~\n19\.\n<\/code><\/pre>/);
  assert.doesNotMatch(other, /19\\\./);

  // Nor does a shorter run of the same delimiter.
  const shorter = renderMarkdown("````text\n```\n19.\n````\n\n19.");
  assert.match(shorter, /<pre><code class="language-text">```\n19\.\n<\/code><\/pre>/);
  assert.match(shorter, />19\.</, "the bare number after the block is still visible text");
  assert.doesNotMatch(shorter, /<ol/);
});
