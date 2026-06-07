import { React } from "../../../lib/html.js";
import { useQuery } from "@tanstack/react-query";
import { fetchJobFiles, readJobFile } from "../lib/jobs-api.js";

function mapEntries(entries = []) {
  return entries.map((entry) => ({
    name: entry.name,
    path: entry.path,
    isDir: entry.is_dir,
    children: entry.is_dir ? [] : null,
    loaded: false,
    expanded: false,
  }));
}

function findNode(nodes, targetPath) {
  for (const node of nodes) {
    if (node.path === targetPath) return node;
    if (node.children?.length) {
      const nested = findNode(node.children, targetPath);
      if (nested) return nested;
    }
  }
  return null;
}

function updateNode(nodes, targetPath, updater) {
  return nodes.map((node) => {
    if (node.path === targetPath) {
      return updater(node);
    }
    if (node.children?.length) {
      return { ...node, children: updateNode(node.children, targetPath, updater) };
    }
    return node;
  });
}

export function useJobFiles(job) {
  const [tree, setTree] = React.useState([]);
  const [selectedPath, setSelectedPath] = React.useState("");
  const [treeError, setTreeError] = React.useState("");
  const [expandingPath, setExpandingPath] = React.useState("");

  const canBrowse = Boolean(job?.project_dir && job?.id);

  const rootQuery = useQuery({
    queryKey: ["job-files-root", job?.id],
    queryFn: () => fetchJobFiles(job.id, ""),
    enabled: canBrowse,
  });

  const fileQuery = useQuery({
    queryKey: ["job-file", job?.id, selectedPath],
    queryFn: () => readJobFile(job.id, selectedPath),
    enabled: Boolean(canBrowse && selectedPath),
  });

  React.useEffect(() => {
    setTree([]);
    setSelectedPath("");
    setTreeError("");
    setExpandingPath("");
  }, [job?.id]);

  React.useEffect(() => {
    if (rootQuery.data?.entries) {
      setTree(mapEntries(rootQuery.data.entries));
      setTreeError("");
    } else if (rootQuery.error) {
      setTreeError(rootQuery.error.message || "Unable to load project files");
    }
  }, [rootQuery.data, rootQuery.error]);

  const toggleDirectory = React.useCallback(
    async (path) => {
      const node = findNode(tree, path);
      if (!node || !job?.id) return;

      if (node.expanded) {
        setTree((current) => updateNode(current, path, (currentNode) => ({ ...currentNode, expanded: false })));
        return;
      }

      if (node.loaded) {
        setTree((current) => updateNode(current, path, (currentNode) => ({ ...currentNode, expanded: true })));
        return;
      }

      setExpandingPath(path);
      try {
        const response = await fetchJobFiles(job.id, path);
        setTree((current) =>
          updateNode(current, path, (currentNode) => ({
            ...currentNode,
            expanded: true,
            loaded: true,
            children: mapEntries(response.entries),
          }))
        );
        setTreeError("");
      } catch (error) {
        setTreeError(error.message || "Unable to open folder");
      } finally {
        setExpandingPath("");
      }
    },
    [job?.id, tree]
  );

  return {
    canBrowse,
    tree,
    selectedPath,
    selectPath: setSelectedPath,
    selectedFile: fileQuery.data || null,
    fileError: fileQuery.error?.message || "",
    isLoadingTree: rootQuery.isLoading,
    isLoadingFile: fileQuery.isLoading || fileQuery.isFetching,
    expandingPath,
    treeError,
    toggleDirectory,
  };
}
