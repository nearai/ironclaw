import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { test, vi } from "vitest";

vi.mock("@tanstack/react-query", () => ({
  useQuery: () => ({ data: { entries: [] }, isLoading: false, isError: false }),
}));

vi.mock("../../../lib/i18n", () => ({
  useI18n: () => ({ lang: "de" }),
  useT: () => (key: string, params: Record<string, string> = {}) => {
    const labels: Record<string, string> = {
      "workspace.area.home": "Start",
      "workspace.area.memory": "Speicher",
      "workspace.fileMeta": "{mime} · {size}",
    };
    return (labels[key] || key)
      .replace("{mime}", params.mime || "")
      .replace("{size}", params.size || "");
  },
}));

vi.mock("../../../lib/api", () => ({
  fetchAttachmentBlob: vi.fn(),
}));

vi.mock("../../../lib/download", () => ({
  saveBlob: vi.fn(),
}));

vi.mock("../../../lib/toast", () => ({
  toast: vi.fn(),
}));

import { fetchAttachmentBlob } from "../../../lib/api";
import { saveBlob } from "../../../lib/download";
import { toast } from "../../../lib/toast";
import { WorkspaceTree } from "./workspace-tree";
import { WorkspaceViewer } from "./workspace-viewer";

type TestNodeProps = {
  children?: React.ReactNode;
  onClick?: () => Promise<void>;
};

function findNode(
  node: React.ReactNode,
  predicate: (node: React.ReactElement<TestNodeProps>) => boolean,
): React.ReactElement<TestNodeProps> | null {
  if (!React.isValidElement<TestNodeProps>(node)) return null;
  if (predicate(node)) return node;
  for (const child of React.Children.toArray(node.props.children)) {
    const match = findNode(child, predicate);
    if (match) return match;
  }
  return null;
}

test("workspace tree renders localized area labels instead of backend ids", () => {
  const html = renderToStaticMarkup(
    <WorkspaceTree
      entries={[
        { name: "workspace", path: "workspace", is_dir: true },
        { name: "memory", path: "memory", is_dir: true },
      ]}
      selectedPath=""
      expandedPaths={new Set()}
      filter=""
      onToggleDirectory={() => {}}
      onSelectFile={() => {}}
      isLoading={false}
    />,
  );

  assert.match(html, />Start</);
  assert.match(html, />Speicher</);
  assert.doesNotMatch(html, />workspace</);
  assert.doesNotMatch(html, />memory</);
});

test("workspace viewer renders a locale-aware human-readable file size", () => {
  const html = renderToStaticMarkup(
    <WorkspaceViewer
      path="workspace/archive.bin"
      file={{
        kind: "binary",
        mime: "application/octet-stream",
        size_bytes: 1536,
        download_path: "/api/webchat/v2/fs/content?mount=workspace&path=archive.bin",
      }}
      isLoading={false}
      onNavigate={() => {}}
    />,
  );

  assert.match(html, /application\/octet-stream · 1,5\s+kB/);
  assert.doesNotMatch(html, /1536/);
});

test("workspace viewer shows a localized error toast when download fails", async () => {
  vi.mocked(fetchAttachmentBlob).mockRejectedValueOnce(new Error("network offline"));
  const setDownloading = vi.fn();
  const useState = vi.spyOn(React, "useState").mockReturnValue([false, setDownloading]);
  const useCallback = vi.spyOn(React, "useCallback").mockImplementation((callback) => callback);

  try {
    const viewer = WorkspaceViewer({
      path: "workspace/archive.bin",
      file: {
        kind: "binary",
        mime: "application/octet-stream",
        size_bytes: 1536,
        download_path: "/api/webchat/v2/fs/content?mount=workspace&path=archive.bin",
      },
      isLoading: false,
      onNavigate: () => {},
    });
    const downloadButton = findNode(
      viewer,
      (node) => node.props.children === "workspace.download",
    );

    assert.ok(downloadButton?.props.onClick, "download button should render");
    await downloadButton.props.onClick();

    assert.deepEqual(setDownloading.mock.calls, [[true], [false]]);
    assert.equal(vi.mocked(saveBlob).mock.calls.length, 0);
    assert.deepEqual(vi.mocked(toast).mock.calls, [
      ["workspace.downloadFailed", { tone: "error" }],
    ]);
  } finally {
    useState.mockRestore();
    useCallback.mockRestore();
  }
});
