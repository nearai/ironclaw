// Click-to-preview modal for a message attachment.
//
// Renders one attachment in a focused modal, choosing a CSP-allowed
// representation per `attachmentPreviewMode`:
//   image/audio/video → inline element fed a `data:` URL (img-src / media-src
//                        'self' data:)
//   pdf               → inline <iframe> fed a `blob:` URL (frame-src 'self' blob:)
//   text-like         → fetched text in a <pre>
//   download          → metadata panel for binaries we won't render inline
// A Download action is always offered. The bytes are fetched once (the
// authenticated byte endpoint can't be hit by a bare element src, which carries
// no bearer); object URLs created for the PDF frame / download are revoked when
// the modal closes.

import { React, html } from "../../../lib/html.js";
import { Modal, ModalBody, ModalFooter, ModalHeader } from "../../../design-system/modal.js";
import { Icon } from "../../../design-system/icons.js";
import { fetchAttachmentBlob, blobToDataUrl } from "../../../lib/api.js";
import { attachmentPreviewMode } from "../lib/attachments.js";

// Cap inline text rendering so a large (but within the byte limit) text file
// can't jank the modal. The full file is still one Download away.
const MAX_TEXT_PREVIEW_CHARS = 100_000;

export function AttachmentPreviewModal({ attachment, onClose }) {
  const open = Boolean(attachment);
  // `view` holds the resolved representation: { dataUrl?, frameUrl?, text?,
  // downloadUrl?, truncated? }. `status` is the load state machine.
  const [status, setStatus] = React.useState("loading");
  const [view, setView] = React.useState({});

  const mode = attachment ? attachmentPreviewMode(attachment.mime_type) : "download";

  React.useEffect(() => {
    if (!attachment) return undefined;
    setStatus("loading");
    setView({});

    // Optimistic (just-sent) image: the local data URL is already renderable
    // and there is nothing landed to fetch yet.
    if (!attachment.fetch_url && attachment.preview_url) {
      setView({ dataUrl: attachment.preview_url, downloadUrl: attachment.preview_url });
      setStatus("ready");
      return undefined;
    }
    if (!attachment.fetch_url) {
      setStatus("error");
      return undefined;
    }

    let cancelled = false;
    let objectUrl = null;
    fetchAttachmentBlob(attachment.fetch_url)
      .then(async (blob) => {
        // The object URL doubles as the download href and the PDF frame src.
        objectUrl = URL.createObjectURL(blob);
        const next = { downloadUrl: objectUrl };
        if (mode === "image" || mode === "audio" || mode === "video") {
          next.dataUrl = await blobToDataUrl(blob);
        } else if (mode === "pdf") {
          next.frameUrl = objectUrl;
        } else if (mode === "text") {
          const text = await blob.text();
          next.truncated = text.length > MAX_TEXT_PREVIEW_CHARS;
          next.text = next.truncated ? text.slice(0, MAX_TEXT_PREVIEW_CHARS) : text;
        }
        if (cancelled) {
          URL.revokeObjectURL(objectUrl);
          return;
        }
        setView(next);
        setStatus("ready");
      })
      .catch(() => {
        if (!cancelled) setStatus("error");
      });

    return () => {
      cancelled = true;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [attachment, mode]);

  if (!attachment) return null;

  const filename = attachment.filename || "attachment";

  return html`
    <${Modal} open=${open} onClose=${onClose} size="xl">
      <${ModalHeader} onClose=${onClose}>
        <span className="block truncate">${filename}</span>
      <//>
      <${ModalBody} className="flex min-h-[12rem] items-center justify-center">
        ${status === "loading" &&
        html`<div className="text-sm text-iron-400">Loading…</div>`}
        ${status === "error" &&
        html`<div className="text-sm text-iron-400">Couldn't load this attachment.</div>`}
        ${status === "ready" &&
        html`<${PreviewBody} mode=${mode} view=${view} filename=${filename} />`}
      <//>
      <${ModalFooter}>
        ${view.downloadUrl &&
        html`<a
          href=${view.downloadUrl}
          download=${filename}
          data-testid="attachment-download"
          className="v2-button inline-flex items-center gap-1.5 rounded-md border border-white/10 px-3 py-1.5 text-xs text-iron-200 hover:border-signal/35 hover:text-white"
        >
          <${Icon} name="download" className="h-3.5 w-3.5" />
          <span>Download</span>
        </a>`}
        <button
          type="button"
          onClick=${onClose}
          className="v2-button rounded-md border border-white/10 px-3 py-1.5 text-xs text-iron-200 hover:border-signal/35 hover:text-white"
        >
          Close
        </button>
      <//>
    <//>
  `;
}

function PreviewBody({ mode, view, filename }) {
  switch (mode) {
    case "image":
      return html`<img
        src=${view.dataUrl}
        alt=${filename}
        className="mx-auto max-h-[70vh] w-auto rounded object-contain"
      />`;
    case "audio":
      return html`<audio controls src=${view.dataUrl} className="w-full" />`;
    case "video":
      return html`<video controls src=${view.dataUrl} className="max-h-[70vh] w-full rounded" />`;
    case "pdf":
      return html`<iframe
        src=${view.frameUrl}
        title=${filename}
        className="h-[70vh] w-full rounded border border-iron-700 bg-white"
      />`;
    case "text":
      return html`<div className="w-full">
        <pre
          className="max-h-[70vh] w-full overflow-auto whitespace-pre-wrap break-words rounded bg-iron-900/60 p-3 text-xs text-iron-200"
        >${view.text}</pre>
        ${view.truncated &&
        html`<div className="mt-2 text-xs text-iron-400">
          Preview truncated — download the file to see the rest.
        </div>`}
      </div>`;
    default:
      // Binary we won't render inline; the Download action in the footer is the
      // way out.
      return html`<div className="flex flex-col items-center gap-2 text-iron-400">
        <${Icon} name="file" className="h-10 w-10 text-signal" />
        <div className="text-sm">This file type can't be previewed.</div>
      </div>`;
  }
}
