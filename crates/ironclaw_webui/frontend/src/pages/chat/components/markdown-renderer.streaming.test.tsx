// @vitest-environment happy-dom
// @ts-nocheck
import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { test, vi } from "vitest";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const { renderMarkdownMock } = vi.hoisted(() => ({
  renderMarkdownMock: vi.fn((content) => `<p>${content}</p>`),
}));

vi.mock("../../../lib/i18n", () => ({ useT: () => (key) => key }));
vi.mock("../../../lib/markdown", () => ({ renderMarkdown: renderMarkdownMock }));
vi.mock("streamdown", async () => {
  const { createElement } = await import("react");
  return {
    Streamdown: ({ children, isAnimating, mode }) =>
      createElement(
        "div",
        {
          "data-animating": String(Boolean(isAnimating)),
          "data-mode": mode,
          "data-testid": "streamdown",
        },
        children,
      ),
  };
});

test("streaming Markdown updates immediately and finalizes through the sanitizer", async () => {
  renderMarkdownMock.mockReset();
  renderMarkdownMock.mockImplementation((content) => `<p>${content}</p>`);
  const { MarkdownRenderer } = await import("./markdown-renderer");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);

  try {
    act(() => {
      root.render(React.createElement(MarkdownRenderer, {
        content: "**first**",
        streaming: true,
      }));
    });
    await act(async () => {
      await Promise.resolve();
    });
    assert.equal(container.textContent, "**first**");
    assert.equal(
      container.querySelector('[data-testid="streamdown"]')?.getAttribute(
        "data-mode",
      ),
      "streaming",
    );
    assert.equal(
      container.querySelector('[data-testid="streamdown"]')?.getAttribute(
        "data-animating",
      ),
      "true",
    );
    assert.equal(renderMarkdownMock.mock.calls.length, 0);

    act(() => {
      root.render(React.createElement(MarkdownRenderer, {
        content: "**second**",
        streaming: true,
      }));
    });
    assert.equal(
      container.textContent,
      "**second**",
      "a committed stream snapshot should not wait for a fixed timer",
    );
    assert.equal(renderMarkdownMock.mock.calls.length, 0);

    act(() => {
      root.render(React.createElement(MarkdownRenderer, {
        content: "**final**",
        streaming: false,
      }));
    });
    assert.equal(
      container.querySelector('[data-testid="streamdown"]')?.getAttribute(
        "data-mode",
      ),
      "static",
      "the already-mounted stream renderer should hold the final snapshot while sanitization loads",
    );
    assert.equal(container.textContent, "**final**");
    await act(async () => {
      await Promise.resolve();
    });
    assert.equal(renderMarkdownMock.mock.calls.length, 1);
    assert.equal(renderMarkdownMock.mock.calls[0][0], "**final**");
    assert.equal(container.innerHTML.includes("<p>**final**</p>"), true);
  } finally {
    act(() => root.unmount());
    container.remove();
  }
});
