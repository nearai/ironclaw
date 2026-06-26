import { React, html } from "../../../lib/html.js";
import { statProjectFile, projectFileContentUrl } from "../../../lib/api.js";
import { AttachmentChip } from "./attachment-chip.js";
import { AttachmentPreviewModal } from "./attachment-preview.js";
import { basename, extractWorkspaceFilePaths, formatSize } from "../lib/project-file-paths.js";

// One chip for an agent-referenced workspace file. Builds an attachment-shaped
// descriptor so it reuses the exact same `AttachmentChip` + preview modal as
// message attachments: clicking previews (image/pdf/text/…) with a Download
// action in the modal footer, instead of a bespoke download-only button.
//
// The descriptor's `fetch_url` points at the bearer-only `/files/content`
// endpoint (same byte-fetch shape `fetchAttachmentBlob` expects). `mime_type`
// and the size label come from a best-effort `stat`; until it resolves the chip
// is still clickable (the modal falls back to a download-only view for an
// unknown type).
function ProjectFileChip({ threadId, path, onPreview }) {
  const [meta, setMeta] = React.useState({ mime_type: "", size_label: "" });

  React.useEffect(() => {
    let active = true;
    statProjectFile({ threadId, path })
      .then((response) => {
        if (!active || !response?.stat) return;
        setMeta({
          mime_type: response.stat.mime_type || "",
          size_label: formatSize(response.stat.size_bytes),
        });
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [threadId, path]);

  const att = {
    filename: basename(path),
    mime_type: meta.mime_type,
    size_label: meta.size_label,
    fetch_url: projectFileContentUrl({ threadId, path }),
  };

  return html`<${AttachmentChip}
    att=${att}
    onPreview=${onPreview}
    testId="project-file-chip"
    dataPath=${path}
    downloadTestId="project-file-download"
  />`;
}

// Render a chip row for every workspace file path referenced in `content`, plus
// a single shared preview modal. Renders nothing when there are no references
// or no thread context.
export function ProjectFileChips({ threadId, content }) {
  const paths = React.useMemo(() => extractWorkspaceFilePaths(content), [content]);
  const [previewAttachment, setPreviewAttachment] = React.useState(null);
  if (!threadId || paths.length === 0) return null;
  return html`
    <div className="mt-2 flex flex-col gap-1.5">
      ${paths.map(
        (path) => html`<${ProjectFileChip}
          key=${path}
          threadId=${threadId}
          path=${path}
          onPreview=${setPreviewAttachment}
        />`
      )}
      <${AttachmentPreviewModal}
        attachment=${previewAttachment}
        onClose=${() => setPreviewAttachment(null)}
      />
    </div>
  `;
}
