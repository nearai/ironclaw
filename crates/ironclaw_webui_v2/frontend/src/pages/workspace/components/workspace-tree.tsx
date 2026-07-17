import { useQuery } from "@tanstack/react-query";
import { Button } from "../../../design-system/button";
import { Skeleton } from "../../../design-system/skeleton";
import { useT } from "../../../lib/i18n";
import { cn } from "../../../utils/cn";
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

function TreeNode({ entry, depth, selectedPath, expandedPaths, filter, onToggleDirectory, onSelectFile }) {
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
      <div>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          onClick={() => {
            // Navigate so the main pane lists this folder, and toggle its
            // expansion in the tree — one click drives both the master (tree)
            // and detail (pane); clicking again collapses it.
            onSelectFile(entry.path);
            onToggleDirectory(entry.path);
          }}
          className={cn(
            "h-auto w-full justify-start gap-2 px-2 py-1.5 text-left",
            selectedPath === entry.path
              ? "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
              : "text-[var(--v2-text)]"
          )}
          style={{ paddingLeft: `${8 + depth * 16}px` }}
          aria-expanded={isExpanded}
        >
          <span className={cn("w-3 text-[10px]", isExpanded && "rotate-90")}>{">"}</span>
          <span className="min-w-0 truncate font-semibold">{displayName}</span>
        </Button>
        {isExpanded && (
          <div className="space-y-1">
            {childQuery.isLoading
              ? (<div className="space-y-1 px-4 py-2"><Skeleton className="h-3 w-24" /><span className="sr-only">{t("workspace.loading")}</span></div>)
              : childQuery.isError
              ? (<div className="px-4 py-2 text-xs text-[var(--v2-danger-text)]">{t("workspace.unableOpenDirectory")}</div>)
              : children.map((child) => (
                  <TreeNode
                    key={child.path}
                    entry={child}
                    depth={depth + 1}
                    selectedPath={selectedPath}
                    expandedPaths={expandedPaths}
                    filter={filter}
                    onToggleDirectory={onToggleDirectory}
                    onSelectFile={onSelectFile}
                  />
                ))}
          </div>
        )}
      </div>
    );
  }

  return (
    <Button
      type="button"
      variant="ghost"
      size="sm"
      onClick={() => onSelectFile(entry.path)}
      className={cn(
        "h-auto w-full justify-start gap-2 px-2 py-1.5 text-left",
        selectedPath === entry.path
          ? "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
          : "text-[var(--v2-text-muted)]"
      )}
      style={{ paddingLeft: `${24 + depth * 16}px` }}
    >
      <span className="min-w-0 truncate">{displayName}</span>
    </Button>
  );
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
  if (isLoading) {
    return (<div className="space-y-2 p-3">{[1, 2, 3, 4].map((i) => (<div key={i} className="v2-skeleton h-8 rounded-md" />))}</div>);
  }

  const rootEntries = sortEntries(
    entries.filter((entry) => !isUiHiddenWorkspacePath(entry.path)),
    (entry) => areaDisplayName(entry.path, t),
  );
  if (!rootEntries.length) {
    return (<div className="px-4 py-8 text-sm text-iron-300">{t("workspace.noFiles")}</div>);
  }

  // Mounts (the tree roots) are always shown so the picker never disappears;
  // the filter narrows their contents as you drill in.
  return (
    <div className="space-y-1 p-2">
      {rootEntries.map((entry) => (
        <TreeNode
          key={entry.path}
          entry={entry}
          depth={0}
          selectedPath={selectedPath}
          expandedPaths={expandedPaths}
          filter={filter}
          onToggleDirectory={onToggleDirectory}
          onSelectFile={onSelectFile}
        />
      ))}
    </div>
  );
}
