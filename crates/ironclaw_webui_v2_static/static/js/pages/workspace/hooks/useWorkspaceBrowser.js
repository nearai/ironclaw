import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { React } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import {
  listWorkspace,
  readWorkspaceFile,
  searchWorkspace,
  writeWorkspaceFile,
} from "../lib/workspace-api.js";

export function useWorkspaceBrowser(selectedPath) {
  const t = useT();
  const queryClient = useQueryClient();
  const [expandedPaths, setExpandedPaths] = React.useState(new Set());
  const [search, setSearch] = React.useState("");
  const [editing, setEditing] = React.useState(false);
  const [draft, setDraft] = React.useState("");
  const [result, setResult] = React.useState(null);

  const rootQuery = useQuery({
    queryKey: ["workspace-list", ""],
    queryFn: () => listWorkspace(""),
  });

  const fileQuery = useQuery({
    queryKey: ["workspace-file", selectedPath],
    queryFn: () => readWorkspaceFile(selectedPath),
    enabled: Boolean(selectedPath),
  });

  const searchQuery = useQuery({
    queryKey: ["workspace-search", search.trim()],
    queryFn: () => searchWorkspace(search.trim(), 20),
    enabled: search.trim().length > 0,
  });

  React.useEffect(() => {
    if (fileQuery.data?.content != null && !editing) {
      setDraft(fileQuery.data.content);
    }
  }, [editing, fileQuery.data?.content]);

  React.useEffect(() => {
    setEditing(false);
    setResult(null);
  }, [selectedPath]);

  const loadDirectory = React.useCallback(
    (path) => queryClient.fetchQuery({
      queryKey: ["workspace-list", path],
      queryFn: () => listWorkspace(path),
    }),
    [queryClient]
  );

  const toggleDirectory = React.useCallback(async (path) => {
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
      setResult({ type: "error", message: error.message || t("workspace.unableOpenDirectory") });
    }
  }, [expandedPaths, loadDirectory]);

  const saveMutation = useMutation({
    mutationFn: () => writeWorkspaceFile({ path: selectedPath, content: draft }),
    onSuccess: () => {
      setEditing(false);
      setResult({ type: "success", message: t("workspace.savedPath", { path: selectedPath }) });
      queryClient.invalidateQueries({ queryKey: ["workspace-file", selectedPath] });
      queryClient.invalidateQueries({ queryKey: ["workspace-list"] });
    },
    onError: (error) => {
      setResult({ type: "error", message: error.message || t("workspace.unableSaveFile") });
    },
  });

  return {
    rootEntries: rootQuery.data?.entries || [],
    file: fileQuery.data || null,
    searchResults: searchQuery.data?.results || [],
    expandedPaths,
    search,
    setSearch,
    editing,
    setEditing,
    draft,
    setDraft,
    result,
    clearResult: () => setResult(null),
    isLoadingTree: rootQuery.isLoading,
    isLoadingFile: fileQuery.isLoading,
    isSearching: searchQuery.isFetching,
    isSaving: saveMutation.isPending,
    error: rootQuery.error || fileQuery.error || searchQuery.error || null,
    loadDirectory,
    toggleDirectory,
    save: saveMutation.mutateAsync,
    refresh: () => {
      queryClient.invalidateQueries({ queryKey: ["workspace-list"] });
      queryClient.invalidateQueries({ queryKey: ["workspace-file", selectedPath] });
    },
  };
}
