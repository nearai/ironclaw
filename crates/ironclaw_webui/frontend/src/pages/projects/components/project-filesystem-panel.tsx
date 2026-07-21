import { useQuery } from "@tanstack/react-query";
import React from "react";
import { Panel, StatusPill } from "../../../design-system/primitives";
import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { useT } from "../../../lib/i18n";
import {
  fetchAttachmentBlob,
  listProjectFiles,
  projectFileContentUrl,
} from "../../../lib/api";

// Single-panel, project-scoped filesystem browser.
//
// The project's files live under its `/workspace` mount, which is reachable per
// thread (`/threads/{id}/files`) — every thread in the project shares the same
// bind-mounted folder, so any project thread lists the project's scoped folder.
// Read-only: navigate directories, download files. There is no tree+viewer
// split — just one directory listing.
const PROJECT_FS_ROOT = "/workspace";

function sortEntries(entries) {
  const dirIsFirst = (entry) => (entry.kind === "directory" ? 0 : 1);
  return [...entries].sort(
    (a, b) =>
      dirIsFirst(a) - dirIsFirst(b) ||
      a.name.localeCompare(b.name, undefined, { sensitivity: "base" })
  );
}

// Path segments under the workspace root, for the breadcrumb.
function relSegments(path) {
  if (!path) return [];
  return String(path)
    .replace(/^\/workspace\/?/, "")
    .split("/")
    .filter(Boolean);
}

export function ProjectFilesystemPanel({ threadId }) {
  const t = useT();
  const tRef = React.useRef(t);
  tRef.current = t;
  const [path, setPath] = React.useState(undefined);
  const [downloadError, setDownloadError] = React.useState(null);

  const listing = useQuery({
    queryKey: ["project-files", threadId || "", path || ""],
    queryFn: () => listProjectFiles({ threadId, path }),
    enabled: Boolean(threadId),
  });

  const entries = React.useMemo(
    () => sortEntries(listing.data?.entries || []),
    [listing.data]
  );

  const openEntry = React.useCallback(
    async (entry) => {
      if (entry.kind === "directory") {
        setDownloadError(null);
        setPath(entry.path);
        return;
      }
      try {
        setDownloadError(null);
        const blob = await fetchAttachmentBlob(
          projectFileContentUrl({ threadId, path: entry.path })
        );
        const url = URL.createObjectURL(blob);
        const anchor = document.createElement("a");
        anchor.href = url;
        anchor.download = entry.name;
        document.body.appendChild(anchor);
        anchor.click();
        anchor.remove();
        URL.revokeObjectURL(url);
      } catch (error) {
        setDownloadError(error?.message || tRef.current("projects.files.downloadError"));
      }
    },
    [threadId]
  );

  const segments = relSegments(path);

  const header = (
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div className="flex items-center gap-2">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">
          {t("projects.files.label")}
        </div>
        <StatusPill tone="muted" label={t("workspace.readOnly")} />
      </div>
      <Button
        variant="secondary"
        size="sm"
        onClick={() => listing.refetch()}
        disabled={!threadId || listing.isFetching}
      >
        {listing.isFetching ? t("workspace.refreshing") : t("workspace.refresh")}
      </Button>
    </div>
  );

  if (!threadId) {
    return (
      <Panel className="p-4 sm:p-5">
        {header}
        <div className="mt-4 rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
          {t("projects.files.noFilesYet")}
        </div>
      </Panel>
    );
  }

  return (
    <Panel className="p-4 sm:p-5">
      {header}

      <div className="mt-3 flex min-w-0 flex-wrap items-center gap-1.5 font-mono text-xs text-iron-400">
        <button
          type="button"
          onClick={() => setPath(undefined)}
          className="text-signal hover:underline"
        >
          {t("projects.files.root")}
        </button>
        {segments.map((segment, index) => {
          const target = `${PROJECT_FS_ROOT}/${segments.slice(0, index + 1).join("/")}`;
          return (
            <React.Fragment key={target}>
              <span className="text-[var(--v2-text-muted)]">/</span>
              <button
                type="button"
                onClick={() => setPath(target)}
                className="max-w-[160px] truncate text-signal hover:underline"
              >
                {segment}
              </button>
            </React.Fragment>
          );
        })}
      </div>

      {downloadError &&
      (
        <div className="mt-3 rounded-xl border border-red-400/30 bg-red-500/10 px-3 py-2 text-xs text-red-200">
          {downloadError}
        </div>
      )}
      {listing.error &&
      (
        <div className="mt-3 rounded-xl border border-red-400/30 bg-red-500/10 px-3 py-2 text-xs text-red-200">
          {listing.error.message}
        </div>
      )}

      <div className="mt-3 space-y-1">
        {listing.isLoading
          ? [1, 2, 3, 4].map(
              (index) => (<div key={index} className="v2-skeleton h-9 rounded-[12px]" />)
            )
          : entries.length
          ? entries.map(
              (entry) => (
                <button
                  key={entry.path}
                  type="button"
                  onClick={() => openEntry(entry)}
                  data-testid="project-filesystem-entry"
                  data-entry-kind={entry.kind}
                  data-entry-path={entry.path}
                  className="flex w-full items-center gap-3 rounded-[12px] border border-transparent px-3 py-2 text-left hover:border-white/10 hover:bg-white/[0.04]"
                >
                  <Icon
                    name={entry.kind === "directory" ? "folder" : "file"}
                    className="h-4 w-4 shrink-0 text-iron-300"
                  />
                  <span className="min-w-0 flex-1 truncate text-sm text-white">{entry.name}</span>
                  {entry.kind === "directory"
                    ? (<Icon name="chevron" className="h-3.5 w-3.5 shrink-0 -rotate-90 text-[var(--v2-text-muted)]" />)
                    : (<Icon name="download" className="h-3.5 w-3.5 shrink-0 text-[var(--v2-text-muted)]" />)}
                </button>
              )
            )
          : (
              <div className="rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                {t("projects.files.folderEmpty")}
              </div>
            )}
      </div>
    </Panel>
  );
}
