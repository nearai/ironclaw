import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, test, vi } from "vitest";

const queryState = vi.hoisted(() => ({
  value: { data: { entries: [] }, isLoading: false, isError: false },
}));

vi.mock("@tanstack/react-query", () => ({
  useQuery: () => queryState.value,
}));

vi.mock("../../../lib/i18n", () => ({
  useI18n: () => ({ lang: "de" }),
  useT: () => (key: string, params: Record<string, string> = {}) => {
    const labels: Record<string, string> = {
      "workspace.area.home": "Start",
      "workspace.area.memory": "Speicher",
      "workspace.fileMeta": "{mime} · {size}",
      "workspace.filterPlaceholder": "Nach Namen filtern…",
      "workspace.pickFileTitle": "Datei aus dem Arbeitsbereich auswählen",
      "workspace.breadcrumbRoot": "Arbeitsbereich",
      "workspace.unableOpenDirectory": "Ordner konnte nicht geöffnet werden",
    };
    return (labels[key] || key)
      .replace("{mime}", params.mime || "")
      .replace("{size}", params.size || "");
  },
}));

import { WorkspaceTree } from "./workspace-tree";
import { WorkspaceBreadcrumb } from "./workspace-breadcrumb";
import { WorkspaceSidebar } from "./workspace-sidebar";
import { WorkspaceViewer } from "./workspace-viewer";

afterEach(() => {
  queryState.value = { data: { entries: [] }, isLoading: false, isError: false };
});

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

test("workspace tree exposes hierarchy, expansion, selection, and roving focus semantics", () => {
  const html = renderToStaticMarkup(
    <WorkspaceTree
      entries={[
        { name: "workspace", path: "workspace", is_dir: true },
        { name: "memory", path: "memory", is_dir: true },
      ]}
      selectedPath="memory"
      expandedPaths={new Set(["memory"])}
      filter=""
      onToggleDirectory={() => {}}
      onSelectFile={() => {}}
      isLoading={false}
    />,
  );

  assert.match(
    html,
    /role="tree" aria-label="Datei aus dem Arbeitsbereich auswählen"/,
  );
  assert.match(
    html,
    /role="treeitem" tabindex="0" aria-label="Speicher" aria-expanded="true" aria-selected="true"[^>]*data-tree-path="memory"/,
  );
  assert.match(html, /role="group"/);
  assert.equal((html.match(/tabindex="0"/g) || []).length, 1);
});

test("workspace filter and breadcrumb have accessible names and landmarks", () => {
  const sidebar = renderToStaticMarkup(
    <WorkspaceSidebar
      rootEntries={[]}
      selectedPath=""
      expandedPaths={new Set()}
      filter=""
      onFilterChange={() => {}}
      isLoadingTree={false}
      onToggleDirectory={() => {}}
      onSelectFile={() => {}}
    />,
  );
  const breadcrumb = renderToStaticMarkup(
    <WorkspaceBreadcrumb path="workspace/reports" onNavigate={() => {}} />,
  );

  assert.match(sidebar, /aria-label="Nach Namen filtern…"/);
  assert.match(breadcrumb, /<nav aria-label="Arbeitsbereich"/);
});

test("workspace tree directory failures are announced as alerts", () => {
  queryState.value = { data: null, isLoading: false, isError: true };
  const html = renderToStaticMarkup(
    <WorkspaceTree
      entries={[{ name: "workspace", path: "workspace", is_dir: true }]}
      selectedPath="workspace"
      expandedPaths={new Set(["workspace"])}
      filter=""
      onToggleDirectory={() => {}}
      onSelectFile={() => {}}
      isLoading={false}
    />,
  );

  assert.match(html, /role="alert"/);
  assert.match(html, /Ordner konnte nicht geöffnet werden/);
});

test("workspace tree announces loading expanded directories", () => {
  queryState.value = { data: null, isLoading: true, isError: false };
  const html = renderToStaticMarkup(
    <WorkspaceTree
      entries={[{ name: "workspace", path: "workspace", is_dir: true }]}
      selectedPath="workspace"
      expandedPaths={new Set(["workspace"])}
      filter=""
      onToggleDirectory={() => {}}
      onSelectFile={() => {}}
      isLoading={false}
    />,
  );

  assert.match(html, /role="treeitem"[^>]*aria-busy="true"/);
  assert.match(html, /role="status"/);
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
