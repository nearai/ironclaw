// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

const rendererSource = readFileSync(
  new URL("./markdown-renderer.tsx", import.meta.url),
  "utf8",
);
const appCssSource = readFileSync(
  new URL("../../../styles/app.css", import.meta.url),
  "utf8",
);

function rendererEnhancerSourceForTest() {
  const lines = [];
  for (const line of rendererSource.split("\n")) {
    if (line.startsWith("import ")) continue;
    if (line.startsWith("function MarkdownRendererImpl")) break;
    lines.push(line);
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { enhanceCodeBlocks };`;
}

class FakeElement {
  constructor(tagName) {
    this.tagName = tagName.toUpperCase();
    this.children = [];
    this.parentNode = null;
    this.dataset = {};
    this.style = {};
    this.listeners = {};
    this.className = "";
    this.textContent = "";
    this.innerText = "";
    this.scrollHeight = 0;
  }

  appendChild(child) {
    if (child.parentNode) {
      child.parentNode.children = child.parentNode.children.filter(
        (item) => item !== child,
      );
    }
    child.parentNode = this;
    this.children.push(child);
    return child;
  }

  insertBefore(child, reference) {
    if (child.parentNode) {
      child.parentNode.children = child.parentNode.children.filter(
        (item) => item !== child,
      );
    }
    child.parentNode = this;
    const index = this.children.indexOf(reference);
    if (index === -1) {
      this.children.push(child);
    } else {
      this.children.splice(index, 0, child);
    }
    return child;
  }

  addEventListener(type, handler) {
    this.listeners[type] = handler;
  }

  querySelectorAll(selector) {
    const matches = [];
    this.#visit((node) => {
      if (selector === "pre" && node.tagName === "PRE") matches.push(node);
    });
    return matches;
  }

  querySelector(selector) {
    let found = null;
    this.#visit((node) => {
      if (found) return;
      if (selector === "code" && node.tagName === "CODE") {
        found = node;
        return;
      }
      const role = selector.match(/^\[data-code-block-role="([^"]+)"\]$/)?.[1];
      if (role && node.dataset.codeBlockRole === role) found = node;
    });
    return found;
  }

  closest(selector) {
    if (selector !== ".markdown-code-frame") return null;
    for (let node = this; node; node = node.parentNode) {
      if (String(node.className).split(/\s+/).includes("markdown-code-frame")) {
        return node;
      }
    }
    return null;
  }

  #visit(visitor) {
    visitor(this);
    for (const child of this.children) {
      child.#visit(visitor);
    }
  }
}

function setupEnhancerContext() {
  const toastCalls = [];
  const clipboardWrites = [];
  const timers = [];
  const context = {
    document: {
      createElement: (tagName) => new FakeElement(tagName),
    },
    window: {},
    navigator: {
      clipboard: {
        writeText: async (text) => {
          clipboardWrites.push(text);
        },
      },
    },
    setTimeout: (fn) => {
      timers.push(fn);
    },
    toast: (...args) => toastCalls.push(args),
    globalThis: {},
  };
  vm.runInNewContext(rendererEnhancerSourceForTest(), context);
  return { clipboardWrites, context, timers, toastCalls };
}

function buildCodeBlock() {
  const root = new FakeElement("div");
  const pre = new FakeElement("pre");
  pre.scrollHeight = 420;
  const code = new FakeElement("code");
  code.innerText = "console.log('hello')";
  pre.appendChild(code);
  root.appendChild(pre);
  return { code, pre, root };
}

function translator(prefix) {
  return (key) =>
    ({
      "common.copy": `${prefix}:copy`,
      "common.copied": `${prefix}:copied`,
      "markdown.codeCopied": `${prefix}:code-copied`,
      "markdown.wrap": `${prefix}:wrap`,
      "markdown.noWrap": `${prefix}:no-wrap`,
      "markdown.showMore": `${prefix}:show-more`,
      "markdown.showLess": `${prefix}:show-less`,
    })[key] || key;
}

test("markdown code blocks are passed through highlight.js when available", () => {
  assert.ok(
    rendererSource.includes('import hljs from "highlight.js/lib/common"'),
    "highlight.js should be bundled through npm rather than loaded from a window global",
  );
  assert.match(
    rendererSource,
    /hljs\.highlightElement\(codeEl\)/,
    "markdown code blocks should be enhanced by highlight.js after rendering",
  );
});

test("markdown code block controls use resynced labels after language changes", async () => {
  const { clipboardWrites, context, timers, toastCalls } = setupEnhancerContext();
  const { pre, root } = buildCodeBlock();
  const { enhanceCodeBlocks } = context.globalThis.__testExports;

  enhanceCodeBlocks(root, translator("old"));
  enhanceCodeBlocks(root, translator("new"));

  const frame = pre.closest(".markdown-code-frame");
  const wrapBtn = frame.querySelector('[data-code-block-role="wrap"]');
  const copyBtn = frame.querySelector('[data-code-block-role="copy"]');
  const expandBtn = frame.querySelector('[data-code-block-role="expand"]');

  assert.equal(wrapBtn.textContent, "new:wrap");
  assert.equal(copyBtn.textContent, "new:copy");
  assert.equal(expandBtn.textContent, "new:show-more");

  wrapBtn.listeners.click();
  assert.equal(wrapBtn.textContent, "new:no-wrap");
  wrapBtn.listeners.click();
  assert.equal(wrapBtn.textContent, "new:wrap");

  await copyBtn.listeners.click();
  assert.deepEqual(clipboardWrites, ["console.log('hello')"]);
  assert.equal(copyBtn.textContent, "new:copied");
  assert.equal(toastCalls[0][0], "new:code-copied");
  assert.equal(toastCalls[0][1].tone, "success");
  timers[0]();
  assert.equal(copyBtn.textContent, "new:copy");

  expandBtn.listeners.click();
  assert.equal(expandBtn.textContent, "new:show-less");
  expandBtn.listeners.click();
  assert.equal(expandBtn.textContent, "new:show-more");
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
