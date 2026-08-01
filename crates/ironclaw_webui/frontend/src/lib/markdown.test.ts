// @vitest-environment happy-dom
import assert from "node:assert/strict";
import { test } from "vitest";
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
