import { React, html } from "../../../lib/html.js";
import { Icon } from "../../../design-system/icons.js";
import { useT } from "../../../lib/i18n.js";
import { toast } from "../../../lib/toast.js";
import { fetchProjectFileBlob, statProjectFile } from "../../../lib/api.js";
import { saveBlob } from "../../../lib/download.js";
import {
  basename,
  extractWorkspaceFilePaths,
  formatSize,
} from "../lib/project-file-paths.js";

function ProjectFileChip({ threadId, path }) {
  const t = useT();
  const [sizeLabel, setSizeLabel] = React.useState("");
  const [downloading, setDownloading] = React.useState(false);

  // Best-effort size for the chip; a stat failure (missing file, race) just
  // leaves the size blank rather than surfacing an error.
  React.useEffect(() => {
    let active = true;
    statProjectFile({ threadId, path })
      .then((response) => {
        if (active && response?.stat) setSizeLabel(formatSize(response.stat.size_bytes));
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [threadId, path]);

  const onDownload = React.useCallback(async () => {
    setDownloading(true);
    try {
      const blob = await fetchProjectFileBlob({ threadId, path });
      saveBlob(blob, basename(path));
    } catch (_) {
      toast(t("chat.fileDownloadFailed"), { tone: "error" });
    } finally {
      setDownloading(false);
    }
  }, [threadId, path, t]);

  return html`
    <button
      type="button"
      data-testid="project-file-chip"
      data-file-path=${path}
      onClick=${onDownload}
      disabled=${downloading}
      title=${path}
      className="flex items-center gap-2 rounded-md border border-iron-700 bg-iron-900/50 px-3 py-2 text-xs text-left hover:border-signal/35 disabled:opacity-60"
    >
      <${Icon} name="file" className="h-3.5 w-3.5 shrink-0 text-signal" />
      <span className="truncate">${basename(path)}</span>
      <span className="ml-auto inline-flex shrink-0 items-center gap-1 text-iron-200">
        ${sizeLabel}
        <${Icon} name="download" className="h-3.5 w-3.5" />
      </span>
    </button>
  `;
}

// Render a chip row for every workspace file path referenced in `content`.
// Renders nothing when there are no references or no thread context.
export function ProjectFileChips({ threadId, content }) {
  const paths = React.useMemo(() => extractWorkspaceFilePaths(content), [content]);
  if (!threadId || paths.length === 0) return null;
  return html`
    <div className="mt-2 flex flex-col gap-1.5">
      ${paths.map(
        (path) => html`<${ProjectFileChip} key=${path} threadId=${threadId} path=${path} />`
      )}
    </div>
  `;
}
