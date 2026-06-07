import { useQuery } from "@tanstack/react-query";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { StatusPill } from "../../../design-system/primitives.js";
import { listWorkspace } from "../lib/workspace-api.js";
import { formatWorkspaceDate, snippetFor } from "../lib/workspace-presenters.js";

function isUiHiddenWorkspacePath(path = "") {
  return String(path)
    .split("/")
    .some((segment) => segment.startsWith("."));
}

function TreeNode({ entry, depth, selectedPath, expandedPaths, onToggleDirectory, onSelectFile }) {
  const t = useT();
  const isExpanded = expandedPaths.has(entry.path);
  const childQuery = useQuery({
    queryKey: ["workspace-list", entry.path],
    queryFn: () => listWorkspace(entry.path),
    enabled: entry.is_dir && isExpanded,
  });

  if (entry.is_dir) {
    return html`
      <div>
        <button
          type="button"
          onClick=${() => onToggleDirectory(entry.path)}
          className="flex min-h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm text-iron-200 hover:bg-white/[0.05] hover:text-white"
          style=${{ paddingLeft: `${8 + depth * 16}px` }}
          aria-expanded=${isExpanded}
        >
          <span className=${["w-3 text-[10px]", isExpanded ? "rotate-90" : ""].join(" ")}>></span>
          <span className="min-w-0 truncate font-semibold">${entry.name}</span>
        </button>
        ${isExpanded && html`
          <div className="space-y-1">
            ${childQuery.isLoading
              ? html`<div className="px-4 py-2 text-xs text-iron-400">${t("workspace.loading")}</div>`
              : (childQuery.data?.entries || [])
                .filter((child) => !isUiHiddenWorkspacePath(child.path))
                .map((child) => html`
                  <${TreeNode}
                    key=${child.path}
                    entry=${child}
                    depth=${depth + 1}
                    selectedPath=${selectedPath}
                    expandedPaths=${expandedPaths}
                    onToggleDirectory=${onToggleDirectory}
                    onSelectFile=${onSelectFile}
                  />
                `)}
          </div>
        `}
      </div>
    `;
  }

  return html`
    <button
      type="button"
      onClick=${() => onSelectFile(entry.path)}
      className=${[
        "flex min-h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm",
        selectedPath === entry.path ? "bg-signal/10 text-signal" : "text-iron-300 hover:bg-white/[0.05] hover:text-white",
      ].join(" ")}
      style=${{ paddingLeft: `${24 + depth * 16}px` }}
    >
      <span className="min-w-0 truncate">${entry.name}</span>
    </button>
  `;
}

export function WorkspaceTree({
  entries,
  selectedPath,
  expandedPaths,
  onToggleDirectory,
  onSelectFile,
  isLoading,
}) {
  const t = useT();
  if (isLoading) {
    return html`<div className="space-y-2 p-3">${[1, 2, 3, 4].map((i) => html`<div key=${i} className="v2-skeleton h-8 rounded-md" />`)}</div>`;
  }

  const visibleEntries = entries.filter((entry) => !isUiHiddenWorkspacePath(entry.path));

  if (!visibleEntries.length) {
    return html`<div className="px-4 py-8 text-sm text-iron-300">${t("workspace.noFiles")}</div>`;
  }

  return html`
    <div className="space-y-1 p-2">
      ${visibleEntries.map((entry) => html`
        <${TreeNode}
          key=${entry.path}
          entry=${entry}
          depth=${0}
          selectedPath=${selectedPath}
          expandedPaths=${expandedPaths}
          onToggleDirectory=${onToggleDirectory}
          onSelectFile=${onSelectFile}
        />
      `)}
    </div>
  `;
}

export function WorkspaceSearchResults({ results, query, onSelectFile, isSearching }) {
  const t = useT();
  if (isSearching) {
    return html`<div className="p-4 text-sm text-iron-300">${t("workspace.searching")}</div>`;
  }

  const visibleResults = results.filter((result) => !isUiHiddenWorkspacePath(result.path));

  if (!visibleResults.length) {
    return html`<div className="p-4 text-sm text-iron-300">${t("workspace.noResults")}</div>`;
  }

  return html`
    <div className="space-y-2 p-2">
      ${visibleResults.map((result) => html`
        <button
          key=${result.path}
          type="button"
          onClick=${() => onSelectFile(result.path)}
          className="w-full rounded-md border border-white/8 bg-white/[0.025] p-3 text-left hover:border-signal/25 hover:bg-white/[0.05]"
        >
          <div className="flex items-center justify-between gap-2">
            <div className="min-w-0 truncate font-mono text-xs text-signal">${result.path}</div>
            <${StatusPill} tone="muted" label=${Number(result.score || 0).toFixed(2)} />
          </div>
          <div className="mt-2 line-clamp-2 text-xs leading-5 text-iron-300">${snippetFor(result.content, query)}</div>
        </button>
      `)}
    </div>
  `;
}
