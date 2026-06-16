// Shared attachment chip + thumbnail, used by both message attachments
// (`message.attachments`) and assistant project-file references
// (`/workspace/...` chips). The chip body opens the shared
// `AttachmentPreviewModal`; a separate trailing download icon saves the bytes
// directly without opening the modal.
//
// Kept generic over the attachment descriptor shape:
//   { filename, mime_type, kind?, size_label?, fetch_url?, preview_url? }
// `fetch_url` is a same-origin relative path the bearer-authenticated
// `fetchAttachmentBlob` can GET (a message-attachment byte URL or a project
// `/files/content?path=` URL). Optional `testId`/`dataPath`/`downloadTestId`
// stamp test hooks so the project-file usage keeps its selectors.

import { React, html } from "../../../lib/html.js";
import { Icon } from "../../../design-system/icons.js";
import { fetchAttachmentBlob, fetchAttachmentDataUrl } from "../../../lib/api.js";
import { saveBlob } from "../../../lib/download.js";

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
  // Lazy init keeps the optimistic-image first render flicker-free; the effect
  // below owns every subsequent (re)sync, including resets, so a chip reused for
  // a different `att` never keeps a stale thumbnail.
  const [resolvedUrl, setResolvedUrl] = React.useState(() =>
    isImage ? att.preview_url || null : null,
  );

  React.useEffect(() => {
    // Non-image, or no source at all: drop any prior thumbnail, show the icon.
    if (!isImage) {
      setResolvedUrl(null);
      return undefined;
    }
    // Optimistic image: the local data URL is already renderable.
    if (att.preview_url) {
      setResolvedUrl(att.preview_url);
      return undefined;
    }
    if (!att.fetch_url) {
      setResolvedUrl(null);
      return undefined;
    }
    // Persisted image: clear any stale thumbnail from a previous `att`, then
    // fetch the authenticated bytes for this one.
    setResolvedUrl(null);
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

/* One attachment chip: thumbnail/icon + filename + type/size, plus a trailing
   download icon. The body opens the preview modal when the attachment has bytes
   to show (a landed `fetch_url`, or an optimistic image's local `preview_url`);
   the download icon (shown only for landed `fetch_url`s) fetches the bearer-
   authenticated bytes and saves them directly. With no bytes at all the chip is
   a static row. */
const ATTACHMENT_CHIP_BASE =
  "flex items-stretch rounded-md border border-iron-700 bg-iron-900/50 text-xs";
const ATTACHMENT_CHIP_PADDING = "px-3 py-2";

export function AttachmentChip({ att, onPreview, testId, dataPath, downloadTestId }) {
  const [downloading, setDownloading] = React.useState(false);

  const onDownload = React.useCallback(async () => {
    if (!att.fetch_url) return;
    setDownloading(true);
    try {
      const blob = await fetchAttachmentBlob(att.fetch_url);
      saveBlob(blob, att.filename || "download");
    } catch (_) {
      /* Best-effort: the preview body still offers its own Download action. */
    } finally {
      setDownloading(false);
    }
  }, [att.fetch_url, att.filename]);

  const inner = html`
    <${AttachmentThumbnail} att=${att} />
    <span className="truncate">${att.filename || "attachment"}</span>
    <span className="ml-auto shrink-0 text-iron-200"
      >${att.mime_type}${att.size_label ? " / " + att.size_label : ""}</span
    >
  `;

  // No bytes to show or save: a plain static row.
  if (!att.fetch_url && !att.preview_url) {
    return html`<div
      className=${`${ATTACHMENT_CHIP_BASE} ${ATTACHMENT_CHIP_PADDING} items-center gap-2`}
      data-testid=${testId}
      data-file-path=${dataPath}
    >
      ${inner}
    </div>`;
  }

  return html`<div className=${`${ATTACHMENT_CHIP_BASE} overflow-hidden`}>
    <button
      type="button"
      onClick=${() => onPreview(att)}
      aria-label=${`Preview ${att.filename || "attachment"}`}
      data-testid=${testId}
      data-file-path=${dataPath}
      className=${`flex min-w-0 flex-1 items-center gap-2 ${ATTACHMENT_CHIP_PADDING} text-left transition-colors hover:bg-iron-900/80`}
    >
      ${inner}
    </button>
    ${att.fetch_url &&
    html`<button
      type="button"
      onClick=${onDownload}
      disabled=${downloading}
      aria-label=${`Download ${att.filename || "attachment"}`}
      data-testid=${downloadTestId}
      className="flex shrink-0 items-center border-l border-iron-700 px-2.5 text-iron-200 transition-colors hover:bg-iron-900/80 hover:text-white disabled:opacity-50"
    >
      <${Icon} name="download" className="h-3.5 w-3.5" />
    </button>`}
  </div>`;
}
