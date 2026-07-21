import { useQuery } from "@tanstack/react-query";
import React from "react";
import { useT } from "../../../lib/i18n";
import { listWorkspace } from "../lib/workspace-api";
import { areaDisplayName, sortEntries } from "../lib/workspace-presenters";

type WorkspaceEntry = {
  name: string;
  path: string;
  is_dir: boolean;
};

type TreeNodeProps = {
  entry: WorkspaceEntry;
  depth: number;
  selectedPath: string;
  expandedPaths: Set<string>;
  filter: string;
  onToggleDirectory: (path: string) => void;
  onSelectFile: (path: string) => void;
  focusedPathRef: React.RefObject<string>;
  registerTreeItem: (path: string, item: HTMLElement | null) => void;
  onTreeItemFocus: (item: HTMLElement) => void;
  onTreeItemKeyDown: (
    event: React.KeyboardEvent<HTMLDivElement>,
    entry: WorkspaceEntry,
    isExpanded: boolean,
  ) => void;
  positionInSet: number;
  siblingCount: number;
};

type WorkspaceTreeProps = {
  entries: WorkspaceEntry[];
  selectedPath: string;
  expandedPaths: Set<string>;
  filter: string;
  onToggleDirectory: (path: string) => void;
  onSelectFile: (path: string) => void;
  isLoading: boolean;
};

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

const TreeNode = React.memo(function WorkspaceTreeNode({
  entry,
  depth,
  selectedPath,
  expandedPaths,
  filter,
  onToggleDirectory,
  onSelectFile,
  focusedPathRef,
  registerTreeItem,
  onTreeItemFocus,
  onTreeItemKeyDown,
  positionInSet,
  siblingCount,
}: TreeNodeProps) {
  const t = useT();
  const treeItemRef = React.useCallback(
    (item) => registerTreeItem(entry.path, item),
    [entry.path, registerTreeItem],
  );
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
        ref={treeItemRef}
        role="treeitem"
        tabIndex={focusedPathRef.current === entry.path ? 0 : -1}
        aria-label={displayName}
        aria-expanded={isExpanded}
        aria-busy={isExpanded && childQuery.isLoading ? true : undefined}
        aria-selected={selectedPath === entry.path}
        aria-level={depth + 1}
        aria-posinset={positionInSet}
        aria-setsize={siblingCount}
        data-testid="workspace-tree-entry"
        data-entry-path={entry.path}
        data-tree-path={entry.path}
        data-tree-depth={depth}
        onFocus={(event) => onTreeItemFocus(event.currentTarget)}
        onClick={(event) => {
          const clickedTreeItem = (event.target as Element).closest('[role="treeitem"]');
          if (clickedTreeItem !== event.currentTarget) return;
          event.currentTarget.focus();
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
              ? (<div role="status" className="px-4 py-2 text-xs text-iron-400">{t("workspace.loading")}</div>)
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
                    focusedPathRef={focusedPathRef}
                    registerTreeItem={registerTreeItem}
                    onTreeItemFocus={onTreeItemFocus}
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
      ref={treeItemRef}
      role="treeitem"
      tabIndex={focusedPathRef.current === entry.path ? 0 : -1}
      aria-label={displayName}
      aria-selected={selectedPath === entry.path}
      aria-level={depth + 1}
      aria-posinset={positionInSet}
      aria-setsize={siblingCount}
      data-testid="workspace-tree-entry"
      data-entry-path={entry.path}
      data-tree-path={entry.path}
      data-tree-depth={depth}
      onFocus={(event) => onTreeItemFocus(event.currentTarget)}
      onClick={(event) => {
        event.currentTarget.focus();
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
});

function directTreeItem(parent: Element, fromEnd = false) {
  let child = fromEnd ? parent.lastElementChild : parent.firstElementChild;
  while (child) {
    if (child.getAttribute("role") === "treeitem") return child as HTMLElement;
    child = fromEnd ? child.previousElementSibling : child.nextElementSibling;
  }
  return null;
}

function directGroup(item: HTMLElement) {
  let child = item.firstElementChild;
  while (child) {
    if (child.getAttribute("role") === "group") return child as HTMLElement;
    child = child.nextElementSibling;
  }
  return null;
}

function firstChildTreeItem(item: HTMLElement) {
  const group = directGroup(item);
  return group ? directTreeItem(group) : null;
}

function deepestVisibleTreeItem(item: HTMLElement) {
  let current = item;
  let child = firstChildTreeItem(current);
  while (child) {
    const group = directGroup(current);
    const lastChild = group ? directTreeItem(group, true) : null;
    if (!lastChild) break;
    current = lastChild;
    child = firstChildTreeItem(current);
  }
  return current;
}

function nextVisibleTreeItem(item: HTMLElement) {
  const child = firstChildTreeItem(item);
  if (child) return child;

  let current = item;
  while (current) {
    let sibling = current.nextElementSibling;
    while (sibling) {
      if (sibling.getAttribute("role") === "treeitem") return sibling as HTMLElement;
      sibling = sibling.nextElementSibling;
    }
    const group = current.parentElement;
    if (group?.getAttribute("role") !== "group") return null;
    const parent = group.parentElement;
    if (parent?.getAttribute("role") !== "treeitem") return null;
    current = parent;
  }
  return null;
}

function previousVisibleTreeItem(item: HTMLElement) {
  let sibling = item.previousElementSibling;
  while (sibling) {
    if (sibling.getAttribute("role") === "treeitem") {
      return deepestVisibleTreeItem(sibling as HTMLElement);
    }
    sibling = sibling.previousElementSibling;
  }
  const group = item.parentElement;
  if (group?.getAttribute("role") !== "group") return null;
  const parent = group.parentElement;
  return parent?.getAttribute("role") === "treeitem" ? parent : null;
}

function parentTreeItem(item: HTMLElement) {
  const group = item.parentElement;
  if (group?.getAttribute("role") !== "group") return null;
  const parent = group.parentElement;
  return parent?.getAttribute("role") === "treeitem" ? parent : null;
}

const useBrowserLayoutEffect =
  typeof window === "undefined" ? React.useEffect : React.useLayoutEffect;

export function WorkspaceTree({
  entries,
  selectedPath,
  expandedPaths,
  filter,
  onToggleDirectory,
  onSelectFile,
  isLoading,
}: WorkspaceTreeProps) {
  const t = useT();
  const rootEntries = sortEntries(
    entries.filter((entry) => !isUiHiddenWorkspacePath(entry.path)),
    (entry) => areaDisplayName(entry.path, t),
  );
  const treeRef = React.useRef<HTMLDivElement>(null);
  const treeItemsByPathRef = React.useRef(new Map<string, HTMLElement>());
  const focusedPathRef = React.useRef(selectedPath || rootEntries[0]?.path || "");
  const focusedItemRef = React.useRef<HTMLElement | null>(null);
  const previousSelectedPathRef = React.useRef(selectedPath);
  const selectedPathRef = React.useRef(selectedPath);
  const pendingSelectedPathRef = React.useRef(selectedPath);
  selectedPathRef.current = selectedPath;

  const setRovingTabStop = React.useCallback((item: HTMLElement, moveDomFocus = false) => {
    const previous = focusedItemRef.current;
    if (previous && previous !== item) previous.tabIndex = -1;
    item.tabIndex = 0;
    focusedItemRef.current = item;
    focusedPathRef.current = item.dataset.treePath || "";
    if (moveDomFocus) item.focus();
  }, []);

  const syncFallbackTabStop = React.useCallback(() => {
    if (focusedItemRef.current) return;
    const selected = treeItemsByPathRef.current.get(selectedPathRef.current);
    const first = treeRef.current ? directTreeItem(treeRef.current) : null;
    const fallback = selected || first;
    if (fallback) setRovingTabStop(fallback);
  }, [setRovingTabStop]);

  const registerTreeItem = React.useCallback(
    (path: string, item: HTMLElement | null) => {
      if (item) {
        treeItemsByPathRef.current.set(path, item);
        if (focusedPathRef.current === path) setRovingTabStop(item);
        if (pendingSelectedPathRef.current === path) {
          pendingSelectedPathRef.current = "";
          setRovingTabStop(item);
        }
        return;
      }

      const registered = treeItemsByPathRef.current.get(path);
      treeItemsByPathRef.current.delete(path);
      if (focusedItemRef.current === registered) {
        focusedItemRef.current = null;
        queueMicrotask(syncFallbackTabStop);
      }
    },
    [setRovingTabStop, syncFallbackTabStop],
  );

  const onTreeItemFocus = React.useCallback(
    (item: HTMLElement) => {
      pendingSelectedPathRef.current = "";
      setRovingTabStop(item);
    },
    [setRovingTabStop],
  );

  const moveTreeFocus = React.useCallback(
    (item: HTMLElement | null) => {
      if (!item) return;
      pendingSelectedPathRef.current = "";
      setRovingTabStop(item, true);
    },
    [setRovingTabStop],
  );

  const onTreeItemKeyDown = React.useCallback(
    (
      event: React.KeyboardEvent<HTMLDivElement>,
      entry: WorkspaceEntry,
      isExpanded: boolean,
    ) => {
      const current = event.currentTarget;

      if (event.key === "ArrowDown") {
        event.preventDefault();
        moveTreeFocus(nextVisibleTreeItem(current));
        return;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        moveTreeFocus(previousVisibleTreeItem(current));
        return;
      }
      if (event.key === "Home") {
        event.preventDefault();
        moveTreeFocus(treeRef.current ? directTreeItem(treeRef.current) : null);
        return;
      }
      if (event.key === "End") {
        event.preventDefault();
        const lastRoot = treeRef.current
          ? directTreeItem(treeRef.current, true)
          : null;
        moveTreeFocus(lastRoot ? deepestVisibleTreeItem(lastRoot) : null);
        return;
      }
      if (event.key === "ArrowRight" && entry.is_dir) {
        event.preventDefault();
        if (!isExpanded) {
          onToggleDirectory(entry.path);
          return;
        }
        // ARIA trees leave focus in place when an open node has no visible
        // child (for example, an empty or currently filtered directory).
        moveTreeFocus(firstChildTreeItem(current));
        return;
      }
      if (event.key === "ArrowLeft") {
        event.preventDefault();
        if (entry.is_dir && isExpanded) {
          onToggleDirectory(entry.path);
          return;
        }
        moveTreeFocus(parentTreeItem(current));
        return;
      }
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        onSelectFile(entry.path);
      }
    },
    [moveTreeFocus, onSelectFile, onToggleDirectory],
  );

  useBrowserLayoutEffect(() => {
    const items = treeItemsByPathRef.current;
    if (!items.size) return;

    const selectedPathChanged = previousSelectedPathRef.current !== selectedPath;
    previousSelectedPathRef.current = selectedPath;
    if (selectedPathChanged) {
      const selected = items.get(selectedPath);
      if (selected) {
        pendingSelectedPathRef.current = "";
        setRovingTabStop(selected);
        return;
      }
      pendingSelectedPathRef.current = selectedPath;
    }

    const focused = items.get(focusedPathRef.current);
    if (focused) {
      setRovingTabStop(focused);
      return;
    }
    focusedItemRef.current = null;
    syncFallbackTabStop();
  }, [
    entries,
    expandedPaths,
    filter,
    selectedPath,
    setRovingTabStop,
    syncFallbackTabStop,
  ]);

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
          focusedPathRef={focusedPathRef}
          registerTreeItem={registerTreeItem}
          onTreeItemFocus={onTreeItemFocus}
          onTreeItemKeyDown={onTreeItemKeyDown}
          positionInSet={index + 1}
          siblingCount={rootEntries.length}
        />
      ))}
    </div>
  );
}
