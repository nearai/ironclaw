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

test("streaming Markdown throttles snapshots and stops after a load failure", async () => {
  vi.useFakeTimers();
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
      await vi.advanceTimersByTimeAsync(0);
    });
    assert.equal(renderMarkdownMock.mock.calls.length, 1);

    act(() => {
      root.render(React.createElement(MarkdownRenderer, {
        content: "**second**",
        streaming: true,
      }));
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(149);
    });
    assert.equal(renderMarkdownMock.mock.calls.length, 1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    assert.equal(renderMarkdownMock.mock.calls.length, 2);

    renderMarkdownMock.mockImplementation(() => {
      throw new Error("markdown chunk failed");
    });
    act(() => {
      root.render(React.createElement(MarkdownRenderer, {
        content: "**failure**",
        streaming: true,
      }));
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(150);
    });
    assert.equal(renderMarkdownMock.mock.calls.length, 3);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    assert.equal(renderMarkdownMock.mock.calls.length, 3);
  } finally {
    act(() => root.unmount());
    container.remove();
    vi.useRealTimers();
  }
});

test("workspace links delegate to the in-app file preview", async () => {
  renderMarkdownMock.mockReset();
  renderMarkdownMock.mockImplementation(
    () =>
      '<p><a href="/workspace/report.csv" data-workspace-path="/workspace/report.csv"><span>report.csv</span></a></p>',
  );
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

    const linkLabel = container.querySelector("a span");
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
