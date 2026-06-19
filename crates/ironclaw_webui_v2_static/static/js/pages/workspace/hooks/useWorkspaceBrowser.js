import { useQuery, useQueryClient } from "@tanstack/react-query";
import { React } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { listWorkspace, readWorkspaceFile } from "../lib/workspace-api.js";

// Read-only browser state for the agent filesystem viewer. The tree is rooted
// at the mount list (empty path); selecting a file loads a preview, selecting a
// folder loads its listing into the main pane. There is intentionally no
// edit/save path — this surface is navigation + preview/download only.
export function useWorkspaceBrowser(selectedPath) {
  const t = useT();
  const queryClient = useQueryClient();
  const [expandedPaths, setExpandedPaths] = React.useState(new Set());
  const [filter, setFilter] = React.useState("");
  const [result, setResult] = React.useState(null);

  const rootQuery = useQuery({
    queryKey: ["workspace-list", ""],
    queryFn: () => listWorkspace(""),
  });

  // Stat/preview of the current selection. For a directory this resolves to
  // `{ kind: "directory" }` (one stat); disabled at the root, which is always
  // a directory.
  const fileQuery = useQuery({
    queryKey: ["workspace-file", selectedPath],
    queryFn: () => readWorkspaceFile(selectedPath),
    enabled: Boolean(selectedPath),
  });

  const selectionIsDirectory =
    selectedPath === "" || fileQuery.data?.kind === "directory";

  // Contents of the selected directory for the main-pane listing. Shares the
  // tree's cache key so an already-expanded folder is served from cache.
  const listingQuery = useQuery({
    queryKey: ["workspace-list", selectedPath],
    queryFn: () => listWorkspace(selectedPath),
    enabled: selectionIsDirectory,
  });

  React.useEffect(() => {
    setResult(null);
  }, [selectedPath]);

  const loadDirectory = React.useCallback(
    (path) =>
      queryClient.fetchQuery({
        queryKey: ["workspace-list", path],
        queryFn: () => listWorkspace(path),
      }),
    [queryClient]
  );

  const toggleDirectory = React.useCallback(
    async (path) => {
      const next = new Set(expandedPaths);
      if (next.has(path)) {
        next.delete(path);
        setExpandedPaths(next);
        return;
      }
      next.add(path);
      setExpandedPaths(next);
      try {
        await loadDirectory(path);
      } catch (error) {
        setResult({
          type: "error",
          message: error.message || t("workspace.unableOpenDirectory"),
        });
      }
    },
    [expandedPaths, loadDirectory, t]
  );

  return {
    rootEntries: rootQuery.data?.entries || [],
    file: fileQuery.data || null,
    selectionIsDirectory,
    currentEntries: listingQuery.data?.entries || [],
    expandedPaths,
    filter,
    setFilter,
    result,
    clearResult: () => setResult(null),
    isLoadingTree: rootQuery.isLoading,
    isLoadingFile: fileQuery.isLoading,
    isLoadingListing: listingQuery.isLoading,
    isFetching:
      rootQuery.isFetching || fileQuery.isFetching || listingQuery.isFetching,
    error: rootQuery.error || fileQuery.error || listingQuery.error || null,
    loadDirectory,
    toggleDirectory,
    refresh: () => {
      queryClient.invalidateQueries({ queryKey: ["workspace-list"] });
      queryClient.invalidateQueries({ queryKey: ["workspace-file", selectedPath] });
    },
  };
}
