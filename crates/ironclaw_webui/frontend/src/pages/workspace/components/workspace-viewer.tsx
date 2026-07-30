import React from "react";
import { Badge, Button, Card, CodePanel, EmptyPanel, Skeleton } from "@ironclaw/ui";
import { useI18n, useT } from "../../../lib/i18n";
import { fetchAttachmentBlob } from "../../../lib/api";
import { saveBlob } from "../../../lib/download";
import { toast } from "../../../lib/toast";
import { MarkdownRenderer } from "../../chat/components/markdown-renderer";
import {
  formatWorkspaceFileSize,
  isMarkdownPath,
  parentPath,
  pathSegments,
} from "../lib/workspace-presenters";
import { WorkspaceBreadcrumb } from "./workspace-breadcrumb";

function fileBaseName(path) {
  return pathSegments(path).pop() || "download";
}

function FileBody({ path, file }) {
  const t = useT();

  if (file.kind === "image") {
    return (
      <div className="flex min-h-0 flex-1 items-start overflow-auto p-4">
        <img
          src={file.image_data_url}
          alt={fileBaseName(path)}
          className="max-h-full max-w-full rounded-lg border border-[var(--v2-panel-border)]"
        />
      </div>
    );
  }

  if (file.kind === "text") {
    return (
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3 sm:px-6 sm:py-4">
        {isMarkdownPath(path)
          ? (<MarkdownRenderer content={file.content} className="max-w-4xl text-base leading-7" />)
          : (<CodePanel wrap>{file.content}</CodePanel>)}
      </div>
    );
  }

  // Binary / unpreviewable: offer a download instead of inlining bytes.
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-4 p-8 text-center">
      <p className="max-w-md text-sm text-[var(--v2-text-muted)]">{t("workspace.binaryPreviewUnavailable")}</p>
    </div>
  );
}

export function WorkspaceViewer({ path, file, isLoading, onNavigate }) {
  const t = useT();
  const { lang } = useI18n();
  const [downloading, setDownloading] = React.useState(false);

  const handleDownload = React.useCallback(async () => {
    if (!file?.download_path) return;
    setDownloading(true);
    try {
      const blob = await fetchAttachmentBlob(file.download_path);
      saveBlob(blob, fileBaseName(path));
    } catch {
      toast(t("workspace.downloadFailed"), { tone: "error" });
    } finally {
      setDownloading(false);
    }
  }, [file, path, t]);

  if (isLoading) {
    return (
      <div className="space-y-4">
        <Skeleton className="h-16" />
        <Skeleton className="h-[460px]" />
      </div>
    );
  }

  if (!file || file.kind === "directory") {
    return (
      <EmptyPanel
        title={t("workspace.pickFileTitle")}
        description={t("workspace.pickFileDesc")}
      />
    );
  }

  const meta = t("workspace.fileMeta", {
    mime: file.mime || "application/octet-stream",
    size: formatWorkspaceFileSize(file.size_bytes, lang),
  });

  return (
    <Card className="flex min-h-[520px] flex-col overflow-hidden xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--v2-panel-border)] px-4 py-3">
        <WorkspaceBreadcrumb path={path} onNavigate={onNavigate} />
        <div className="flex items-center gap-2">
          <Badge tone="muted" label={meta} />
          <Button
            data-testid="workspace-download"
            variant="secondary"
            size="sm"
            onClick={handleDownload}
            disabled={downloading}
          >{t("workspace.download")}</Button>
        </div>
      </div>

      <FileBody path={path} file={file} />

      {parentPath(path) && (
        <div className="border-t border-[var(--v2-panel-border)] px-4 py-3 text-xs text-[var(--v2-text-muted)]">
          {t("workspace.parent", { path: parentPath(path) })}
        </div>
      )}
    </Card>
  );
}
