// @vitest-environment happy-dom
import assert from "node:assert/strict";
import { test } from "vitest";

import {
  highlightCodeBlocks,
  SUPPORTED_LANGUAGES,
} from "./syntax-highlighting";

const EXPECTED_SUPPORTED_LANGUAGES = [
  "bash",
  "c",
  "cpp",
  "csharp",
  "css",
  "diff",
  "go",
  "java",
  "javascript",
  "json",
  "kotlin",
  "markdown",
  "python",
  "ruby",
  "rust",
  "sql",
  "typescript",
  "xml",
  "yaml",
];

test("highlight registry matches the supported language allowlist", () => {
  assert.deepEqual(
    Object.keys(SUPPORTED_LANGUAGES).sort(),
    [...EXPECTED_SUPPORTED_LANGUAGES].sort(),
  );
});

test("highlights every language in the implementation registry", () => {
  const root = document.createElement("div");
  const languageNames = Object.keys(SUPPORTED_LANGUAGES);
  const codeBlocks = languageNames.map((language) => {
    const pre = document.createElement("pre");
    const code = document.createElement("code");
    code.className = `language-${language}`;
    code.textContent = "example";
    pre.append(code);
    root.append(pre);
    return code;
  });

  highlightCodeBlocks(root);

  for (const [index, code] of codeBlocks.entries()) {
    assert.equal(
      code.dataset.highlighted,
      "yes",
      `expected ${languageNames[index]} to be highlighted`,
    );
  }
});
