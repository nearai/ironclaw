// Shared attachment chip + thumbnail, used by both message attachments
// (`message.attachments`) and assistant project-file references
// (`/workspace/...` chips). Clicking a chip with bytes to show opens the shared
// `AttachmentPreviewModal`; the chip otherwise renders as a static row.
//
// Kept generic over the attachment descriptor shape:
//   { filename, mime_type, kind?, size_label?, fetch_url?, preview_url? }
// `fetch_url` is a same-origin relative path the bearer-authenticated
// `fetchAttachmentBlob` can GET (a message-attachment byte URL or a project
// `/files/content?path=` URL). Optional `testId`/`dataPath` stamp test hooks so
// the project-file usage keeps its `data-testid`/`data-file-path` selectors.

import { React, html } from "../../../lib/html.js";
import { Icon } from "../../../design-system/icons.js";
import { fetchAttachmentDataUrl } from "../../../lib/api.js";

/* Thumbnail for one attachment. An optimistic (just-sent) image carries a local
   data URL in `preview_url` and renders immediately. A persisted image instead
   carries a `fetch_url`: `<img>` cannot send the session bearer, so the bytes
   are fetched here and turned into a data URL (the SPA's CSP allows `data:`
   images, not `blob:`). Anything else — non-images, unlanded refs, or a failed
   fetch — falls back to the file icon. */
export function AttachmentThumbnail({ att }) {
  // Only images get a rendered thumbnail. Every landed attachment carries a
  // `fetch_url` (for click-to-preview of any kind), so the thumbnail must gate
  // on kind — otherwise a PDF/text would be fetched and shown as a broken
  // `<img>`. Non-images keep the file icon.
  const isImage =
    att.kind === "image" || (att.mime_type || "").toLowerCase().startsWith("image/");
  const [resolvedUrl, setResolvedUrl] = React.useState(
    isImage ? att.preview_url || null : null,
  );

  React.useEffect(() => {
    if (!isImage || att.preview_url || !att.fetch_url) return undefined;
    // The local data URL is already renderable; only a persisted image needs
    // the authenticated byte fetch.
    let cancelled = false;
    fetchAttachmentDataUrl(att.fetch_url)
      .then((url) => {
        if (!cancelled) setResolvedUrl(url);
      })
      .catch(() => {
        /* Leave the file-icon fallback in place on any read failure. */
      });
    return () => {
      cancelled = true;
    };
  }, [isImage, att.preview_url, att.fetch_url]);

  if (isImage && resolvedUrl) {
    return html`<img
      src=${resolvedUrl}
      alt=${att.filename || "attachment"}
      className="h-9 w-9 shrink-0 rounded object-cover"
    />`;
  }
  return html`<${Icon} name="file" className="h-3.5 w-3.5 shrink-0 text-signal" />`;
}

/* One attachment chip: thumbnail/icon + filename + type/size. Clicking opens
   the preview modal when the attachment has bytes to show (a landed
   `fetch_url`, or an optimistic image's local `preview_url`); otherwise it
   renders as a static row. */
const ATTACHMENT_CHIP_CLASS =
  "flex items-center gap-2 rounded-md border border-iron-700 bg-iron-900/50 px-3 py-2 text-xs";

export function AttachmentChip({ att, onPreview, testId, dataPath }) {
  const inner = html`
    <${AttachmentThumbnail} att=${att} />
    <span className="truncate">${att.filename || "attachment"}</span>
    <span className="ml-auto shrink-0 text-iron-200"
      >${att.mime_type}${att.size_label ? " / " + att.size_label : ""}</span
    >
  `;
  if (!att.fetch_url && !att.preview_url) {
    return html`<div
      className=${ATTACHMENT_CHIP_CLASS}
      data-testid=${testId}
      data-file-path=${dataPath}
    >
      ${inner}
    </div>`;
  }
  return html`<button
    type="button"
    onClick=${() => onPreview(att)}
    aria-label=${`Preview ${att.filename || "attachment"}`}
    data-testid=${testId}
    data-file-path=${dataPath}
    className=${`${ATTACHMENT_CHIP_CLASS} w-full text-left transition-colors hover:border-signal/40 hover:bg-iron-900/80`}
  >
    ${inner}
  </button>`;
}
