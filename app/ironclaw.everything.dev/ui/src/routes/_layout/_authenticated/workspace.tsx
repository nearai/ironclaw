import { createFileRoute } from "@tanstack/react-router";
import {
  ChevronRight,
  FileIcon,
  FolderIcon,
  FolderTree,
  Loader2,
  RefreshCw,
  XCircle,
} from "lucide-react";
import { useEffect, useState } from "react";
import { Card, Skeleton } from "@/components";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { ScrollArea } from "@/components/ui/scroll-area";
import { mountsQueryOptions, useFilesystemMounts, useFsContent, useFsList } from "@/hooks/use-fs";

export const Route = createFileRoute("/_layout/_authenticated/workspace")({
  loader: async ({ context }) => {
    await context.queryClient.ensureQueryData(mountsQueryOptions(context.apiClient));
  },
  component: WorkspacePage,
});

function PathBreadcrumb({ path, onNavigate }: { path: string; onNavigate: (path: string) => void }) {
  const segments = path.split("/").filter(Boolean);
  return (
    <div className="flex items-center gap-1 text-sm text-muted-foreground flex-wrap">
      <button
        type="button"
        onClick={() => onNavigate("")}
        className="hover:text-foreground transition-colors"
      >
        /
      </button>
      {segments.map((seg, i) => {
        const segPath = "/" + segments.slice(0, i + 1).join("/");
        return (
          <span key={segPath} className="flex items-center gap-1">
            <ChevronRight size={12} className="shrink-0" />
            <button
              type="button"
              onClick={() => onNavigate(segPath)}
              className="hover:text-foreground transition-colors truncate max-w-[160px]"
            >
              {seg}
            </button>
          </span>
        );
      })}
    </div>
  );
}

function WorkspacePage() {
  const { data: mounts, isLoading: mountsLoading, isError: mountsError } = useFilesystemMounts();
  const [mount, setMount] = useState<string>("");
  const [path, setPath] = useState("");
  const [selectedFile, setSelectedFile] = useState<string | null>(null);

  useEffect(() => {
    if (mounts && mounts.length > 0 && !mount) {
      setMount(mounts[0].mount);
    }
  }, [mounts, mount]);

  const {
    data: dirData,
    isLoading: dirLoading,
    isError: dirError,
    refetch: refetchDir,
  } = useFsList(mount, path);

  const {
    data: fileContent,
    isLoading: fileLoading,
    isError: fileError,
  } = useFsContent(mount, selectedFile ?? "");

  const entries = dirData?.entries ?? [];

  const handleMountChange = (value: string) => {
    setMount(value);
    setPath("");
    setSelectedFile(null);
  };

  const handleNavigate = (targetPath: string) => {
    setPath(targetPath);
    setSelectedFile(null);
  };

  const handleFileClick = (filePath: string) => {
    setSelectedFile(filePath);
  };

  if (mountsLoading) {
    return (
      <div className="p-6 space-y-4">
        <Skeleton className="h-10 w-48" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (mountsError || !mounts || mounts.length === 0) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <div className="rounded-lg border border-border p-8 text-center space-y-3 max-w-sm">
          <div className="flex h-12 w-12 items-center justify-center rounded-full bg-muted mx-auto">
            <FolderTree className="h-6 w-6 text-muted-foreground" />
          </div>
          <p className="text-sm font-medium text-foreground">No mounts available</p>
          <p className="text-xs text-muted-foreground">
            The filesystem browser is not configured or the agent is not running.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-3 px-6 py-3 border-b border-border shrink-0">
        <div className="flex h-8 w-8 items-center justify-center rounded-full bg-primary/10">
          <FolderTree className="h-4 w-4 text-primary" />
        </div>
        <h1 className="text-sm font-semibold text-foreground">Workspace Files</h1>
        <div className="w-px h-5 bg-border mx-1" />
        <Select value={mount} onValueChange={handleMountChange}>
          <SelectTrigger className="h-8 w-[160px] text-xs gap-1.5">
            <FolderIcon size={13} className="text-muted-foreground shrink-0" />
            <SelectValue placeholder="Select mount" />
          </SelectTrigger>
          <SelectContent>
            {mounts.map((m) => (
              <SelectItem key={m.mount} value={m.mount} className="text-sm">
                {m.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <div className="w-px h-5 bg-border" />
        <PathBreadcrumb path={path} onNavigate={handleNavigate} />
      </div>

      <div className="flex flex-1 min-h-0">
        <div className="w-72 shrink-0 border-r border-border flex flex-col min-h-0">
          <ScrollArea className="flex-1">
            {dirLoading ? (
              <div className="p-3 space-y-1.5">
                {Array.from({ length: 8 }).map((_, i) => (
                  <Skeleton key={i} className="h-8 w-full" />
                ))}
              </div>
            ) : dirError ? (
              <div className="p-4 text-center space-y-2">
                <XCircle className="mx-auto h-5 w-5 text-destructive" />
                <p className="text-xs text-destructive">Failed to load directory</p>
                <Button variant="outline" size="sm" onClick={() => refetchDir()}>
                  <RefreshCw size={12} className="mr-1" />
                  Retry
                </Button>
              </div>
            ) : entries.length === 0 ? (
              <div className="p-4 text-center">
                <p className="text-xs text-muted-foreground">Empty directory</p>
              </div>
            ) : (
              <div className="py-1">
                {entries.map((entry) => {
                  const isDir = entry.kind === "directory";
                  return (
                    <button
                      key={entry.path}
                      type="button"
                      onClick={() =>
                        isDir ? handleNavigate(entry.path) : handleFileClick(entry.path)
                      }
                      className={`w-full flex items-center gap-2 px-3 py-1.5 text-sm hover:bg-muted transition-colors text-left ${
                        selectedFile === entry.path
                          ? "bg-muted text-foreground"
                          : "text-muted-foreground"
                      }`}
                    >
                      {isDir ? (
                        <FolderIcon size={14} className="shrink-0 text-amber-500" />
                      ) : (
                        <FileIcon size={14} className="shrink-0" />
                      )}
                      <span className="truncate">{entry.name}</span>
                    </button>
                  );
                })}
              </div>
            )}
          </ScrollArea>
        </div>

        <div className="flex-1 min-w-0 flex flex-col min-h-0">
          {selectedFile ? (
            <ScrollArea className="flex-1">
              {fileLoading ? (
                <div className="flex items-center justify-center h-full">
                  <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
                </div>
              ) : fileError ? (
                <div className="flex items-center justify-center h-full p-6">
                  <div className="text-center space-y-2">
                    <XCircle className="mx-auto h-5 w-5 text-destructive" />
                    <p className="text-xs text-destructive">Failed to load file</p>
                  </div>
                </div>
              ) : fileContent ? (
                <div className="p-6 space-y-3">
                  <div className="flex items-center gap-4 text-xs text-muted-foreground">
                    <span className="inline-flex items-center gap-1.5">
                      <FileIcon size={13} />
                      {selectedFile.split("/").pop()}
                    </span>
                    <span>{fileContent.mimeType}</span>
                    <span>{(fileContent.sizeBytes / 1024).toFixed(1)} KB</span>
                  </div>
                  {(() => {
                    const isText =
                      fileContent.mimeType.startsWith("text/") ||
                      fileContent.mimeType.includes("json") ||
                      fileContent.mimeType.includes("javascript") ||
                      fileContent.mimeType.includes("xml");
                    return (
                      <Card className="p-4">
                        {isText && fileContent.contentBase64 ? (
                          <pre className="text-sm font-mono whitespace-pre-wrap break-all text-foreground max-h-[60vh] overflow-y-auto">
                            {atob(fileContent.contentBase64).slice(0, 50_000)}
                          </pre>
                        ) : (
                          <p className="text-sm text-muted-foreground">
                            Preview not available for {fileContent.mimeType}
                          </p>
                        )}
                      </Card>
                    );
                  })()}
                </div>
              ) : null}
            </ScrollArea>
          ) : (
            <div className="flex items-center justify-center h-full">
              <div className="text-center space-y-2">
                <FileIcon className="mx-auto h-8 w-8 text-muted-foreground" />
                <p className="text-sm text-muted-foreground">Select a file to preview</p>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
