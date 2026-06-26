import { useQuery } from "@tanstack/react-query";
import { React, html } from "../../../lib/html.js";
import { Panel, StatusPill } from "../../../design-system/primitives.js";
import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { useT } from "../../../lib/i18n.js";
import {
  fetchAttachmentBlob,
  listProjectFiles,
  projectFileContentUrl,
} from "../../../lib/api.js";

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
        setDownloadError(error?.message || "Unable to download file");
      }
    },
    // `t` is not referenced in this callback; depending on it would recreate
    // the handler on every locale change for no reason.
    [threadId]
  );

  const segments = relSegments(path);

  const header = html`
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div className="flex items-center gap-2">
        <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">
          ${"Files"}
        </div>
        <${StatusPill} tone="muted" label=${t("workspace.readOnly")} />
      </div>
      <${Button}
        variant="secondary"
        size="sm"
        onClick=${() => listing.refetch()}
        disabled=${!threadId || listing.isFetching}
      >
        ${listing.isFetching ? t("workspace.refreshing") : t("workspace.refresh")}
      <//>
    </div>
  `;

  if (!threadId) {
    return html`
      <${Panel} className="p-4 sm:p-5">
        ${header}
        <div className="mt-4 rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
          ${"No files yet — they appear once a thread has run in this project."}
        </div>
      <//>
    `;
  }

  return html`
    <${Panel} className="p-4 sm:p-5">
      ${header}

      <div className="mt-3 flex min-w-0 flex-wrap items-center gap-1.5 font-mono text-xs text-iron-400">
        <button
          type="button"
          onClick=${() => setPath(undefined)}
          className="text-signal hover:underline"
        >
          ${"workspace"}
        </button>
        ${segments.map((segment, index) => {
          const target = `${PROJECT_FS_ROOT}/${segments.slice(0, index + 1).join("/")}`;
          return html`
            <span key=${target} className="text-iron-500">/</span>
            <button
              key=${`${target}-button`}
              type="button"
              onClick=${() => setPath(target)}
              className="max-w-[160px] truncate text-signal hover:underline"
            >
              ${segment}
            </button>
          `;
        })}
      </div>

      ${downloadError &&
      html`
        <div className="mt-3 rounded-xl border border-red-400/30 bg-red-500/10 px-3 py-2 text-xs text-red-200">
          ${downloadError}
        </div>
      `}
      ${listing.error &&
      html`
        <div className="mt-3 rounded-xl border border-red-400/30 bg-red-500/10 px-3 py-2 text-xs text-red-200">
          ${listing.error.message}
        </div>
      `}

      <div className="mt-3 space-y-1">
        ${listing.isLoading
          ? [1, 2, 3, 4].map(
              (index) => html`<div key=${index} className="v2-skeleton h-9 rounded-[12px]" />`
            )
          : entries.length
          ? entries.map(
              (entry) => html`
                <button
                  key=${entry.path}
                  type="button"
                  onClick=${() => openEntry(entry)}
                  className="flex w-full items-center gap-3 rounded-[12px] border border-transparent px-3 py-2 text-left hover:border-white/10 hover:bg-white/[0.04]"
                >
                  <${Icon}
                    name=${entry.kind === "directory" ? "folder" : "file"}
                    className="h-4 w-4 shrink-0 text-iron-300"
                  />
                  <span className="min-w-0 flex-1 truncate text-sm text-white">${entry.name}</span>
                  ${entry.kind === "directory"
                    ? html`<${Icon} name="chevron" className="h-3.5 w-3.5 shrink-0 -rotate-90 text-iron-500" />`
                    : html`<${Icon} name="download" className="h-3.5 w-3.5 shrink-0 text-iron-500" />`}
                </button>
              `
            )
          : html`
              <div className="rounded-[16px] border border-dashed border-white/10 px-4 py-8 text-sm leading-6 text-iron-300">
                ${"This folder is empty."}
              </div>
            `}
      </div>
    <//>
  `;
}
