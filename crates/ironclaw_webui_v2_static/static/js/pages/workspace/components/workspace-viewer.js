import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { Button } from "../../../design-system/button.js";
import { EmptyPanel, Panel, StatusPill } from "../../../design-system/primitives.js";
import { fetchAttachmentBlob } from "../../../lib/api.js";
import { saveBlob } from "../../../lib/download.js";
import { MarkdownRenderer } from "../../chat/components/markdown-renderer.js";
import { isMarkdownPath, parentPath, pathSegments } from "../lib/workspace-presenters.js";
import { WorkspaceBreadcrumb } from "./workspace-breadcrumb.js";

function fileBaseName(path) {
  return pathSegments(path).pop() || "download";
}

function FileBody({ path, file }) {
  const t = useT();

  if (file.kind === "image") {
    return html`
      <div className="flex min-h-0 flex-1 items-start overflow-auto p-4">
        <img
          src=${file.image_data_url}
          alt=${fileBaseName(path)}
          className="max-h-full max-w-full rounded-lg border border-white/10"
        />
      </div>
    `;
  }

  if (file.kind === "text") {
    return html`
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3 sm:px-6 sm:py-4">
        ${isMarkdownPath(path)
          ? html`<${MarkdownRenderer} content=${file.content} className="max-w-4xl text-base leading-7" />`
          : html`<pre className="overflow-x-auto whitespace-pre-wrap font-mono text-sm leading-6 text-iron-200">${file.content}</pre>`}
      </div>
    `;
  }

  // Binary / unpreviewable: offer a download instead of inlining bytes.
  return html`
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-4 p-8 text-center">
      <p className="max-w-md text-sm text-iron-300">${t("workspace.binaryPreviewUnavailable")}</p>
    </div>
  `;
}

export function WorkspaceViewer({ path, file, isLoading, onNavigate }) {
  const t = useT();
  const [downloading, setDownloading] = React.useState(false);

  const handleDownload = React.useCallback(async () => {
    if (!file?.download_path) return;
    setDownloading(true);
    try {
      const blob = await fetchAttachmentBlob(file.download_path);
      saveBlob(blob, fileBaseName(path));
    } catch {
      // Best-effort download; the tree/hook surface load errors separately.
    } finally {
      setDownloading(false);
    }
  }, [file, path]);

  if (isLoading) {
    return html`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;
  }

  if (!file || file.kind === "directory") {
    return html`
      <${EmptyPanel}
        title=${t("workspace.pickFileTitle")}
        description=${t("workspace.pickFileDesc")}
      />
    `;
  }

  const meta = t("workspace.fileMeta", {
    mime: file.mime || "application/octet-stream",
    size: Number(file.size_bytes || 0),
  });

  return html`
    <${Panel} className="flex min-h-[520px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-4 py-3">
        <${WorkspaceBreadcrumb} path=${path} onNavigate=${onNavigate} />
        <div className="flex items-center gap-2">
          <${StatusPill} tone="muted" label=${meta} />
          <${Button}
            variant="secondary"
            size="sm"
            onClick=${handleDownload}
            disabled=${downloading}
          >${t("workspace.download")}<//>
        </div>
      </div>

      <${FileBody} path=${path} file=${file} />

      ${parentPath(path) && html`
        <div className="border-t border-white/10 px-4 py-3 text-xs text-iron-400">
          ${t("workspace.parent", { path: parentPath(path) })}
        </div>
      `}
    <//>
  `;
}
