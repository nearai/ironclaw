// Attachment staging helpers for the WebChat v2 composer.
//
// The browser stages files locally (reads them to base64 + a preview data
// URL), validates them against the server-advertised inline-attachment
// contract (`session.attachments` — see `useAttachmentConfig`), and hands
// `useChat.send` the wire shape `WebUiInboundAttachment` expects:
// `{ mime_type, filename, data_base64 }`. The server-side decode in
// `webui_inbound::decode_attachments` remains the sole authority — these
// client checks are UX hints that fail fast before a doomed upload.

let stagedSeq = 0;

// Default budgets used only as a fallback when the session has not yet
// resolved the server contract. They mirror `MAX_INLINE_*` in
// `crates/ironclaw_product_workflow/src/webui_inbound.rs`; the server still
// re-validates, so a drift here only changes how early the UX warns.
export const FALLBACK_ATTACHMENT_LIMITS = {
  accept: [],
  maxCount: 10,
  maxFileBytes: 5 * 1024 * 1024,
  maxTotalBytes: 10 * 1024 * 1024,
};

export function attachmentKindFromMime(mime) {
  const normalized = (mime || "").toLowerCase();
  if (normalized.startsWith("image/")) return "image";
  if (normalized.startsWith("audio/")) return "audio";
  return "document";
}

// How the preview modal should render an attachment, derived from its MIME
// type (the kind enum is too coarse — a "document" may be a PDF, CSV, or
// arbitrary binary). Each mode maps to a CSP-allowed representation:
//   image    → <img src=data:>           (img-src 'self' data:)
//   audio    → <audio src=data:>         (media-src 'self' data:)
//   video    → <video src=data:>         (media-src 'self' data:)
//   pdf      → <iframe src=blob:>        (frame-src 'self' blob:)
//   text     → fetched text in <pre>     (no media src needed)
//   download → metadata + download link  (binary we won't render inline)
export function attachmentPreviewMode(mime) {
  const normalized = (mime || "").toLowerCase();
  if (normalized.startsWith("image/")) return "image";
  if (normalized.startsWith("audio/")) return "audio";
  if (normalized.startsWith("video/")) return "video";
  if (normalized === "application/pdf") return "pdf";
  if (isTextLikeMime(normalized)) return "text";
  return "download";
}

// Text-renderable MIME types: anything `text/*` plus the common structured
// text formats that are not served as `text/*` (JSON/XML and their `+json` /
// `+xml` suffixes, CSV).
function isTextLikeMime(normalized) {
  return (
    normalized.startsWith("text/") ||
    normalized === "application/json" ||
    normalized === "application/xml" ||
    normalized === "application/csv" ||
    normalized.endsWith("+json") ||
    normalized.endsWith("+xml")
  );
}

export function formatBytes(bytes) {
  if (!Number.isFinite(bytes) || bytes < 0) return "";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const rounded = value >= 10 || Number.isInteger(value)
    ? Math.round(value)
    : Math.round(value * 10) / 10;
  return `${rounded} ${units[unit]}`;
}

// `accept` token matching, mirroring the browser's native file-input
// semantics: `image/*` / `audio/*` wildcards, exact MIME tokens, and
// `.ext` tokens matched against the filename. An empty accept list (e.g.
// the fallback before the session resolves) accepts everything and defers
// entirely to the server.
export function isAcceptedFile(file, accept) {
  if (!accept || accept.length === 0) return true;
  const mime = (file.type || "").toLowerCase();
  const name = (file.name || "").toLowerCase();
  return accept.some((token) => {
    const t = token.trim().toLowerCase();
    if (!t) return false;
    if (t === "*/*" || t === "*") return true;
    if (t.endsWith("/*")) return mime.startsWith(t.slice(0, -1));
    if (t.startsWith(".")) return name.endsWith(t);
    return mime === t;
  });
}

function readAsDataUrl(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      // `readAsDataURL` yields a string; guard the contract so a non-string /
      // null result rejects here (surfaced as `chat.attachmentReadFailed`)
      // rather than crashing `splitDataUrl`'s `.indexOf` downstream.
      if (typeof reader.result === "string") {
        resolve(reader.result);
      } else {
        reject(new Error("file read produced no data URL"));
      }
    };
    reader.onerror = () => reject(reader.error || new Error("file read failed"));
    reader.readAsDataURL(file);
  });
}

// Split a `data:` URL into its MIME type and base64 payload. FileReader
// emits `data:<mime>;base64,<payload>`; an empty/typeless file yields
// `data:;base64,...`, so we fall back to the File's own `type`.
function splitDataUrl(dataUrl, fallbackMime) {
  const comma = dataUrl.indexOf(",");
  if (comma < 0) return { mime: fallbackMime || "", base64: "" };
  const header = dataUrl.slice(0, comma);
  const base64 = dataUrl.slice(comma + 1);
  const match = header.match(/^data:([^;]*)/);
  const mime = (match && match[1]) || fallbackMime || "";
  return { mime, base64 };
}

/**
 * Stage a list of `File`s against the attachment contract.
 *
 * Returns `{ staged, errors }`:
 * - `staged`: array of `{ id, filename, mimeType, kind, sizeBytes, sizeLabel,
 *   dataBase64, previewUrl }` ready to render and send.
 * - `errors`: i18n message keys (with params) for files that were rejected,
 *   so the caller can surface a single combined notice.
 *
 * `existing` is the already-staged list so count/total budgets account for
 * what is already attached.
 */
export async function stageFiles(files, { limits, existing = [], t }) {
  const cfg = limits || FALLBACK_ATTACHMENT_LIMITS;
  const staged = [];
  const errors = [];
  let count = existing.length;
  let total = existing.reduce((sum, att) => sum + (att.sizeBytes || 0), 0);

  for (const file of files) {
    if (count >= cfg.maxCount) {
      errors.push(t("chat.attachmentTooMany", { max: cfg.maxCount }));
      break;
    }
    if (!isAcceptedFile(file, cfg.accept)) {
      errors.push(
        t("chat.attachmentUnsupportedType", { name: file.name || "file" }),
      );
      continue;
    }
    if (file.size > cfg.maxFileBytes) {
      errors.push(
        t("chat.attachmentTooLarge", {
          name: file.name || "file",
          max: formatBytes(cfg.maxFileBytes),
        }),
      );
      continue;
    }
    if (total + file.size > cfg.maxTotalBytes) {
      // A later, smaller file may still fit the remaining budget, so skip this
      // one rather than abandoning the rest of the selection. De-dup the notice
      // so several oversized files don't stack identical messages.
      const err = t("chat.attachmentTotalTooLarge", {
        max: formatBytes(cfg.maxTotalBytes),
      });
      if (!errors.includes(err)) {
        errors.push(err);
      }
      continue;
    }

    let dataUrl;
    try {
      dataUrl = await readAsDataUrl(file);
    } catch {
      errors.push(t("chat.attachmentReadFailed", { name: file.name || "file" }));
      continue;
    }
    const { mime, base64 } = splitDataUrl(dataUrl, file.type);
    const mimeType = mime || "application/octet-stream";
    const kind = attachmentKindFromMime(mimeType);
    staged.push({
      id: `staged-${stagedSeq++}`,
      filename: file.name || "attachment",
      mimeType,
      kind,
      sizeBytes: file.size,
      sizeLabel: formatBytes(file.size),
      dataBase64: base64,
      // Only images carry a preview URL; it is the full data URL so the
      // composer can render a thumbnail without a byte-fetch round trip.
      previewUrl: kind === "image" ? dataUrl : null,
    });
    count += 1;
    total += file.size;
  }

  return { staged, errors };
}

// Map a staged attachment into the `WebUiInboundAttachment` wire shape the
// v2 send-message endpoint accepts.
export function toWireAttachment(att) {
  return {
    mime_type: att.mimeType,
    filename: att.filename,
    data_base64: att.dataBase64,
  };
}

// Map a staged attachment into the in-thread render shape used by
// `MessageBubble` (so an optimistic message shows cards/thumbnails
// immediately, matching what the timeline projection later returns).
export function toRenderAttachment(att) {
  return {
    id: att.id,
    filename: att.filename,
    mime_type: att.mimeType,
    kind: att.kind,
    size_label: att.sizeLabel,
    preview_url: att.previewUrl,
  };
}
