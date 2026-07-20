import { useQuery } from "@tanstack/react-query";
import React from "react";
import { useT } from "../../../lib/i18n";
import { listWorkspace } from "../lib/workspace-api";
import { areaDisplayName, sortEntries } from "../lib/workspace-presenters";

function isUiHiddenWorkspacePath(path = "") {
  return String(path)
    .split("/")
    .some((segment) => segment.startsWith("."));
}

// Narrow a level's entries by the name filter. Hidden (".") paths are always
// dropped. With a filter active, a non-matching directory is still kept when
// it is expanded, so an already-drilled branch stays reachable to its matching
// descendants rather than vanishing mid-path. Filtering only sees loaded
// levels — it does not auto-expand to search unloaded subtrees.
function visibleEntries(entries, filter, expandedPaths) {
  const needle = String(filter || "").trim().toLowerCase();
  const filtered = (entries || [])
    .filter((entry) => !isUiHiddenWorkspacePath(entry.path))
    .filter((entry) => {
      if (!needle) return true;
      if (entry.name.toLowerCase().includes(needle)) return true;
      return entry.is_dir && expandedPaths.has(entry.path);
    });
  // Same folders-first, alphabetical order the main listing uses.
  return sortEntries(filtered);
}

function TreeNode({
  entry,
  depth,
  selectedPath,
  expandedPaths,
  filter,
  onToggleDirectory,
  onSelectFile,
  focusedPath,
  setFocusedPath,
  onTreeItemKeyDown,
  positionInSet,
  siblingCount,
}) {
  const t = useT();
  const isExpanded = expandedPaths.has(entry.path);
  const displayName = depth === 0 ? areaDisplayName(entry.path, t) : entry.name;
  const childQuery = useQuery({
    queryKey: ["workspace-list", entry.path],
    queryFn: () => listWorkspace(entry.path),
    enabled: entry.is_dir && isExpanded,
  });

  if (entry.is_dir) {
    const children = visibleEntries(childQuery.data?.entries, filter, expandedPaths);
    return (
      <div
        role="treeitem"
        tabIndex={focusedPath === entry.path ? 0 : -1}
        aria-label={displayName}
        aria-expanded={isExpanded}
        aria-selected={selectedPath === entry.path}
        aria-level={depth + 1}
        aria-posinset={positionInSet}
        aria-setsize={siblingCount}
        data-tree-path={entry.path}
        data-tree-depth={depth}
        onFocus={() => setFocusedPath(entry.path)}
        onClick={(event) => {
          const clickedTreeItem = (event.target as Element).closest('[role="treeitem"]');
          if (clickedTreeItem !== event.currentTarget) return;
          event.currentTarget.focus();
          setFocusedPath(entry.path);
          // Navigate so the main pane lists this folder, and toggle its
          // expansion in the tree — one click drives both the master (tree)
          // and detail (pane); clicking again collapses it.
          onSelectFile(entry.path);
          onToggleDirectory(entry.path);
        }}
        onKeyDown={(event) => {
          if (event.target !== event.currentTarget) return;
          onTreeItemKeyDown(event, entry, isExpanded);
        }}
        className="outline-none focus-visible:ring-2 focus-visible:ring-signal/70 focus-visible:ring-inset"
      >
        <div
          className={[
            "flex min-h-8 w-full cursor-pointer items-center gap-2 rounded-md px-2 text-left text-sm hover:bg-white/[0.05] hover:text-white",
            selectedPath === entry.path
              ? "bg-signal/10 text-signal shadow-[inset_2px_0_0_currentColor]"
              : "text-iron-200",
          ].join(" ")}
          style={{ paddingLeft: `${8 + depth * 16}px` }}
        >
          <span aria-hidden="true" className={["w-3 text-[10px]", isExpanded ? "rotate-90" : ""].join(" ")}>{">"}</span>
          <span className="min-w-0 truncate font-semibold">{displayName}</span>
        </div>
        {isExpanded && (
          <div role="group" className="space-y-1">
            {childQuery.isLoading
              ? (<div className="px-4 py-2 text-xs text-iron-400">{t("workspace.loading")}</div>)
              : childQuery.isError
              ? (<div role="alert" className="px-4 py-2 text-xs text-red-300">{t("workspace.unableOpenDirectory")}</div>)
              : children.map((child, index) => (
                  <TreeNode
                    key={child.path}
                    entry={child}
                    depth={depth + 1}
                    selectedPath={selectedPath}
                    expandedPaths={expandedPaths}
                    filter={filter}
                    onToggleDirectory={onToggleDirectory}
                    onSelectFile={onSelectFile}
                    focusedPath={focusedPath}
                    setFocusedPath={setFocusedPath}
                    onTreeItemKeyDown={onTreeItemKeyDown}
                    positionInSet={index + 1}
                    siblingCount={children.length}
                  />
                ))}
          </div>
        )}
      </div>
    );
  }

  return (
    <div
      role="treeitem"
      tabIndex={focusedPath === entry.path ? 0 : -1}
      aria-label={displayName}
      aria-selected={selectedPath === entry.path}
      aria-level={depth + 1}
      aria-posinset={positionInSet}
      aria-setsize={siblingCount}
      data-tree-path={entry.path}
      data-tree-depth={depth}
      onFocus={() => setFocusedPath(entry.path)}
      onClick={(event) => {
        event.currentTarget.focus();
        setFocusedPath(entry.path);
        onSelectFile(entry.path);
      }}
      onKeyDown={(event) => onTreeItemKeyDown(event, entry, false)}
      className={[
        "flex min-h-8 w-full cursor-pointer items-center gap-2 rounded-md px-2 text-left text-sm outline-none focus-visible:ring-2 focus-visible:ring-signal/70 focus-visible:ring-inset",
        selectedPath === entry.path
          ? "bg-signal/10 text-signal shadow-[inset_2px_0_0_currentColor]"
          : "text-iron-300 hover:bg-white/[0.05] hover:text-white",
      ].join(" ")}
      style={{ paddingLeft: `${24 + depth * 16}px` }}
    >
      <span className="min-w-0 truncate">{displayName}</span>
    </div>
  );
}

function focusTreeItem(items, index, setFocusedPath) {
  const item = items[index];
  if (!item) return;
  setFocusedPath(item.dataset.treePath || "");
  item.focus();
}

export function WorkspaceTree({
  entries,
  selectedPath,
  expandedPaths,
  filter,
  onToggleDirectory,
  onSelectFile,
  isLoading,
}) {
  const t = useT();
  const treeRef = React.useRef<HTMLDivElement>(null);
  const previousSelectedPathRef = React.useRef(selectedPath);
  const [focusedPath, setFocusedPath] = React.useState("");

  const rootEntries = sortEntries(
    entries.filter((entry) => !isUiHiddenWorkspacePath(entry.path)),
    (entry) => areaDisplayName(entry.path, t),
  );
  const effectiveFocusedPath = focusedPath || rootEntries[0]?.path || "";

  const onTreeItemKeyDown = React.useCallback(
    (event, entry, isExpanded) => {
      const items = Array.from(
        treeRef.current?.querySelectorAll<HTMLElement>('[role="treeitem"]') || [],
      );
      const currentIndex = items.indexOf(event.currentTarget);
      if (currentIndex < 0) return;

      if (event.key === "ArrowDown") {
        event.preventDefault();
        focusTreeItem(items, Math.min(currentIndex + 1, items.length - 1), setFocusedPath);
        return;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        focusTreeItem(items, Math.max(currentIndex - 1, 0), setFocusedPath);
        return;
      }
      if (event.key === "Home") {
        event.preventDefault();
        focusTreeItem(items, 0, setFocusedPath);
        return;
      }
      if (event.key === "End") {
        event.preventDefault();
        focusTreeItem(items, items.length - 1, setFocusedPath);
        return;
      }
      if (event.key === "ArrowRight" && entry.is_dir) {
        event.preventDefault();
        if (!isExpanded) {
          onToggleDirectory(entry.path);
          return;
        }
        const next = items[currentIndex + 1];
        const currentDepth = Number(event.currentTarget.dataset.treeDepth);
        if (next && Number(next.dataset.treeDepth) > currentDepth) {
          focusTreeItem(items, currentIndex + 1, setFocusedPath);
        }
        return;
      }
      if (event.key === "ArrowLeft") {
        event.preventDefault();
        if (entry.is_dir && isExpanded) {
          onToggleDirectory(entry.path);
          return;
        }
        const parentDepth = Number(event.currentTarget.dataset.treeDepth) - 1;
        for (let index = currentIndex - 1; index >= 0; index -= 1) {
          if (Number(items[index].dataset.treeDepth) === parentDepth) {
            focusTreeItem(items, index, setFocusedPath);
            break;
          }
        }
        return;
      }
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        onSelectFile(entry.path);
      }
    },
    [onSelectFile, onToggleDirectory],
  );

  React.useEffect(() => {
    const items = Array.from(
      treeRef.current?.querySelectorAll<HTMLElement>('[role="treeitem"]') || [],
    );
    if (!items.length) return;

    const selectedPathChanged = previousSelectedPathRef.current !== selectedPath;
    previousSelectedPathRef.current = selectedPath;
    if (selectedPathChanged) {
      const selected = items.find((item) => item.dataset.treePath === selectedPath);
      if (selected) {
        setFocusedPath(selectedPath);
        return;
      }
    }

    if (items.some((item) => item.dataset.treePath === focusedPath)) return;
    const selected = items.find((item) => item.dataset.treePath === selectedPath);
    setFocusedPath(selected?.dataset.treePath || items[0].dataset.treePath || "");
  }, [entries, expandedPaths, filter, focusedPath, selectedPath]);

  if (isLoading) {
    return (<div className="space-y-2 p-3">{[1, 2, 3, 4].map((i) => (<div key={i} className="v2-skeleton h-8 rounded-md" />))}</div>);
  }

  if (!rootEntries.length) {
    return (<div className="px-4 py-8 text-sm text-iron-300">{t("workspace.noFiles")}</div>);
  }

  // Mounts (the tree roots) are always shown so the picker never disappears;
  // the filter narrows their contents as you drill in.
  return (
    <div
      ref={treeRef}
      role="tree"
      aria-label={t("workspace.pickFileTitle")}
      className="space-y-1 p-2"
    >
      {rootEntries.map((entry, index) => (
        <TreeNode
          key={entry.path}
          entry={entry}
          depth={0}
          selectedPath={selectedPath}
          expandedPaths={expandedPaths}
          filter={filter}
          onToggleDirectory={onToggleDirectory}
          onSelectFile={onSelectFile}
          focusedPath={effectiveFocusedPath}
          setFocusedPath={setFocusedPath}
          onTreeItemKeyDown={onTreeItemKeyDown}
          positionInSet={index + 1}
          siblingCount={rootEntries.length}
        />
      ))}
    </div>
  );
}
