// @vitest-environment happy-dom
// @ts-nocheck
import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { test, vi } from "vitest";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const { fetchAttachmentBlobMock, navigateMock } = vi.hoisted(() => ({
  fetchAttachmentBlobMock: vi.fn(),
  navigateMock: vi.fn(),
}));

vi.mock("react-router", () => ({
  useNavigate: () => navigateMock,
}));
vi.mock("../../../lib/i18n", () => ({
  useT: () => (key) => key,
}));
vi.mock("../../../lib/api", () => ({
  fetchAttachmentBlob: fetchAttachmentBlobMock,
  blobToDataUrl: vi.fn(),
}));
vi.mock("../../../lib/download", () => ({
  saveBlob: vi.fn(),
}));
vi.mock("../../../design-system/icons", () => ({
  Icon: () => null,
}));
vi.mock("../../../design-system/modal", () => ({
  Modal: ({ children }) => <div data-testid="modal">{children}</div>,
  ModalBody: ({ children }) => <main>{children}</main>,
  ModalFooter: ({ children }) => <footer>{children}</footer>,
  ModalHeader: ({ children }) => <header>{children}</header>,
}));

test("fetched HTML stays inert when descriptor metadata claims it is a PDF", async () => {
  fetchAttachmentBlobMock.mockResolvedValueOnce(
    new Blob(
      ['<script>globalThis.__attachmentPreviewExecuted = true</script><h1>safe source</h1>'],
      { type: "text/html" },
    ),
  );
  vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:test-html");
  vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});

  const { AttachmentPreviewModal } = await import("./attachment-preview");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);

  try {
    await act(async () => {
      root.render(<AttachmentPreviewModal
        attachment={{
          filename: "untrusted.html",
          mime_type: "application/pdf",
          fetch_url: "/api/test/untrusted",
          workspace_path: "/workspace/../secret.txt",
        }}
        onClose={() => {}}
      />);
      await Promise.resolve();
      await Promise.resolve();
    });

    assert.equal(container.querySelector("iframe"), null);
    assert.equal(container.querySelector("script"), null);
    assert.equal(
      container.querySelector('[data-testid="attachment-open-workspace"]'),
      null,
    );
    assert.equal(
      container.querySelector("pre")?.textContent,
      '<script>globalThis.__attachmentPreviewExecuted = true</script><h1>safe source</h1>',
    );
    assert.equal(globalThis.__attachmentPreviewExecuted, undefined);
  } finally {
    act(() => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  }
});

test("workspace attachments can open their selected file in the workspace viewer", async () => {
  fetchAttachmentBlobMock.mockResolvedValueOnce(
    new Blob(["test"], { type: "text/plain" }),
  );
  vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:test-workspace");
  vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});
  navigateMock.mockReset();
  const onClose = vi.fn();

  const { AttachmentPreviewModal } = await import("./attachment-preview");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);

  try {
    await act(async () => {
      root.render(<AttachmentPreviewModal
        attachment={{
          filename: "test.txt",
          mime_type: "text/plain",
          fetch_url: "/api/test/workspace",
          workspace_path:
            "/workspace/attachments/2026-07-30/message-1-test.txt",
        }}
        onClose={onClose}
        threadId="thread-project-beta"
      />);
      await Promise.resolve();
      await Promise.resolve();
    });

    const button = container.querySelector(
      '[data-testid="attachment-open-workspace"]',
    );
    assert.ok(button);
    act(() => button.click());

    assert.equal(onClose.mock.calls.length, 1);
    assert.deepEqual(navigateMock.mock.calls, [[
      "/workspace/thread/thread-project-beta/workspace/attachments/2026-07-30/message-1-test.txt",
    ]]);
  } finally {
    act(() => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  }
});

test("failed workspace attachments do not offer an unavailable workspace jump", async () => {
  fetchAttachmentBlobMock.mockRejectedValueOnce(new Error("not found"));
  const { AttachmentPreviewModal } = await import("./attachment-preview");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);

  try {
    await act(async () => {
      root.render(<AttachmentPreviewModal
        attachment={{
          filename: "missing.html",
          mime_type: "text/html",
          fetch_url: "/api/test/missing",
          workspace_path: "/workspace/generated/missing.html",
        }}
        onClose={() => {}}
      />);
      await Promise.resolve();
      await Promise.resolve();
    });

    assert.equal(container.textContent?.includes("chat.attachmentLoadFailed"), true);
    assert.equal(
      container.querySelector('[data-testid="attachment-open-workspace"]'),
      null,
    );
  } finally {
    act(() => root.unmount());
    container.remove();
  }
});
