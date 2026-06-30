import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const rendererSource = readFileSync(
  new URL("./markdown-renderer.js", import.meta.url),
  "utf8",
);
const appCssSource = readFileSync(
  new URL("../../../../styles/app.css", import.meta.url),
  "utf8",
);

test("markdown code blocks are passed through highlight.js when available", () => {
  assert.match(
    rendererSource,
    /window\.hljs\.highlightElement\(codeEl\)/,
    "markdown code blocks should be enhanced by highlight.js after rendering",
  );
});

test("highlight.js token classes have local readable styles", () => {
  for (const selector of [
    ".markdown-body pre code.hljs",
    ".markdown-body .hljs-keyword",
    ".markdown-body .hljs-string",
    ".markdown-body .hljs-comment",
    ".markdown-body .hljs-title",
  ]) {
    assert.ok(
      appCssSource.includes(selector),
      `expected local syntax-highlighting style for ${selector}`,
    );
  }
  assert.match(
    appCssSource,
    /\.markdown-body\s+pre\s+code\s*\{\s*color:\s*var\(--v2-text\);\s*\}/,
    "plain or unknown-language code blocks should remain readable without highlighted tokens",
  );
});
