// @vitest-environment jsdom
// @ts-nocheck
import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { test, vi } from "vitest";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

vi.mock("../../../lib/i18n", () => ({ useT: () => (key) => key }));
vi.mock("streamdown", async () => {
  const { createElement } = await import("react");
  return {
    defaultRemarkPlugins: {},
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

async function waitForMarkdownRender(predicate, message) {
  const deadline = Date.now() + 2_000;
  while (!predicate()) {
    if (Date.now() >= deadline) {
      assert.fail(message);
    }
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10));
    });
  }
}

test("streaming Markdown updates immediately and finalizes through the sanitizer", async () => {
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
    await waitForMarkdownRender(
      () => container.querySelector('[data-testid="streamdown"]') !== null,
      "the streaming renderer should mount",
    );
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
    await waitForMarkdownRender(
      () => container.innerHTML.includes("<strong>final</strong>"),
      "the final snapshot should render through the sanitizer",
    );
  } finally {
    act(() => root.unmount());
    container.remove();
  }
});

test("workspace links delegate to the in-app file preview", async () => {
  const onWorkspaceFileOpen = vi.fn();
  const { MarkdownRenderer } = await import("./markdown-renderer");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);

  try {
    await act(async () => {
      root.render(React.createElement(MarkdownRenderer, {
        content: "[report.csv](/workspace/report.csv)",
        onWorkspaceFileOpen,
      }));
    });
    await waitForMarkdownRender(
      () => container.querySelector("a") !== null,
      "the workspace link should render",
    );

    const linkLabel = container.querySelector("a");
    assert.ok(linkLabel);
    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    linkLabel.dispatchEvent(click);

    assert.equal(click.defaultPrevented, true);
    assert.deepEqual(onWorkspaceFileOpen.mock.calls, [["/workspace/report.csv"]]);
  } finally {
    act(() => root.unmount());
    container.remove();
  }
});

test("workspace links re-render when preview capability becomes available", async () => {
  const onWorkspaceFileOpen = vi.fn();
  const { MarkdownRenderer } = await import("./markdown-renderer");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);

  try {
    await act(async () => {
      root.render(<MarkdownRenderer
        content="[report.csv](/workspace/report.csv)"
      />);
    });
    await waitForMarkdownRender(
      () => container.querySelector("a") !== null,
      "the ordinary link should render",
    );
    assert.equal(
      container.querySelector("a")?.hasAttribute("data-workspace-path"),
      false,
    );

    await act(async () => {
      root.render(<MarkdownRenderer
        content="[report.csv](/workspace/report.csv)"
        onWorkspaceFileOpen={onWorkspaceFileOpen}
      />);
    });
    await waitForMarkdownRender(
      () => container.querySelector("a[data-workspace-path]") !== null,
      "the link should gain preview metadata",
    );

    const link = container.querySelector("a[data-workspace-path]");
    assert.ok(link);
    link.dispatchEvent(new MouseEvent("click", {
      bubbles: true,
      cancelable: true,
    }));
    assert.deepEqual(onWorkspaceFileOpen.mock.calls, [["/workspace/report.csv"]]);
  } finally {
    act(() => root.unmount());
    container.remove();
  }
});

test("workspace links reject forged preview metadata", async () => {
  const onWorkspaceFileOpen = vi.fn();
  const { MarkdownRenderer } = await import("./markdown-renderer");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);

  try {
    await act(async () => {
      root.render(React.createElement(MarkdownRenderer, {
        content:
          '<a href="https://example.com" data-workspace-path="/workspace/secret.txt">external</a>',
        onWorkspaceFileOpen,
      }));
    });
    await waitForMarkdownRender(
      () => container.querySelector("a") !== null,
      "the external link should render",
    );

    const linkLabel = container.querySelector("a");
    assert.ok(linkLabel);
    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    linkLabel.dispatchEvent(click);

    assert.equal(click.defaultPrevented, false);
    assert.deepEqual(onWorkspaceFileOpen.mock.calls, []);
  } finally {
    act(() => root.unmount());
    container.remove();
  }
});

test("workspace links reject matching non-workspace href metadata", async () => {
  const onWorkspaceFileOpen = vi.fn();
  const { MarkdownRenderer } = await import("./markdown-renderer");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);

  try {
    await act(async () => {
      root.render(<MarkdownRenderer
        content={'<a href="https://example.com" data-workspace-path="https://example.com">external</a>'}
        onWorkspaceFileOpen={onWorkspaceFileOpen}
      />);
    });
    await waitForMarkdownRender(
      () => container.querySelector("a") !== null,
      "the external link should render",
    );

    const link = container.querySelector("a");
    assert.ok(link);
    const click = new MouseEvent("click", {
      bubbles: true,
      cancelable: true,
    });
    link.dispatchEvent(click);

    assert.equal(click.defaultPrevented, false);
    assert.deepEqual(onWorkspaceFileOpen.mock.calls, []);
  } finally {
    act(() => root.unmount());
    container.remove();
  }
});
