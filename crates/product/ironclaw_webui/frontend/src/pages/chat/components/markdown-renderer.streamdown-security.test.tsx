// @vitest-environment happy-dom
// @ts-nocheck
import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { test, vi } from "vitest";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

vi.mock("../../../lib/i18n", () => ({ useT: () => (key) => key }));

async function waitForRenderedEmoji(container: HTMLElement) {
  const deadline = Date.now() + 2_000;
  while (!container.textContent?.includes("👋")) {
    if (Date.now() >= deadline) {
      assert.fail("the streaming renderer should render the gemoji shortcode");
    }
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10));
    });
  }
}

test("streaming Markdown renders gemoji shortcodes without rewriting code", async () => {
  const { MarkdownRenderer } = await import("./markdown-renderer");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);

  try {
    act(() => {
      root.render(React.createElement(MarkdownRenderer, {
        content: "Hello :wave: :smile: :sunglasses: :+1:\n\n`:wave:`\n\n```text\n:wave:\n```",
        streaming: true,
      }));
    });
    await waitForRenderedEmoji(container);

    assert.match(container.textContent || "", /Hello 👋 😄 😎 👍/);
    assert.match(container.textContent || "", /:wave:/);
  } finally {
    act(() => root.unmount());
    container.remove();
  }
});

test("streaming Markdown rejects executable model-authored HTML and URLs", async () => {
  const { MarkdownRenderer } = await import("./markdown-renderer");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);

  try {
    act(() => {
      root.render(React.createElement(MarkdownRenderer, {
        content: [
          '<img src="x" onerror="globalThis.__streamdownXss = true">',
          '[unsafe](javascript:globalThis.__streamdownXss=true)',
          "<script>globalThis.__streamdownXss = true</script>",
          "**safe text remains**",
        ].join("\n\n"),
        streaming: true,
      }));
    });
    await act(async () => {
      await Promise.resolve();
    });

    assert.equal(container.querySelector("script"), null);
    assert.equal(container.querySelector("[onerror]"), null);
    assert.equal(
      Array.from(container.querySelectorAll("a")).some((link) =>
        link.getAttribute("href")?.toLowerCase().startsWith("javascript:")
      ),
      false,
    );
    assert.match(container.textContent || "", /safe text remains/);
    assert.equal(globalThis.__streamdownXss, undefined);
  } finally {
    act(() => root.unmount());
    container.remove();
    delete globalThis.__streamdownXss;
  }
});
