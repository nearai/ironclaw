// Read-only filesystem-viewer API client.
//
// Wraps the WebChat v2 `/fs/*` endpoints (backed by the Reborn
// `FilesystemBrowseReader` port) as the path-oriented surface the workspace
// tree/viewer consume. A "qualified path" used throughout the UI is
// `"<mount>/<mount-relative-path>"` — the first segment selects the mount
// (memory/workspace/…), the rest is the path within it. The empty qualified
// path is the root, which lists the available mounts as top-level directories,
// so the tree itself doubles as the mount picker. Strictly read-only: there is
// no write/save path here.

import { apiFetch, fetchAttachmentBlob, fetchAttachmentDataUrl } from "../../../lib/api";

const FS_BASE = "/api/webchat/v2/fs";

// Largest payload we will inline as text in the viewer. Anything larger is
// offered as a download instead of being read into the page.
const MAX_INLINE_TEXT_BYTES = 1024 * 1024;

// Largest image we will fetch and base64-expand into a data URL for inline
// preview. Above this, offer a download instead so a huge image can't hang the
// tab by being read into memory.
const MAX_INLINE_IMAGE_BYTES = 8 * 1024 * 1024;

function splitQualified(qualifiedPath) {
  const segments = String(qualifiedPath || "")
    .split("/")
    .filter(Boolean);
  const mount = segments.shift() || "";
  return { mount, path: segments.join("/") };
}

function joinQualified(mount, relativePath) {
  return relativePath ? `${mount}/${relativePath}` : mount;
}

function isTextLikeMime(mime) {
  const value = String(mime || "").toLowerCase();
  return (
    value.startsWith("text/") ||
    value === "application/json" ||
    value === "application/javascript" ||
    value === "application/xml" ||
    value.endsWith("+json") ||
    value.endsWith("+xml")
  );
}

function isImageMime(mime) {
  return String(mime || "")
    .toLowerCase()
    .startsWith("image/");
}

// Mimes we never try to render as text — skip the sniff fetch and offer a
// download straight away. Everything else (including `application/octet-stream`,
// which is what an unknown extension like `Dockerfile.worker` resolves to) is
// sniffed, so extensionless/unknown text files still preview.
function isLikelyBinaryMime(mime) {
  const value = String(mime || "").toLowerCase();
  return (
    value.startsWith("audio/") ||
    value.startsWith("video/") ||
    value.startsWith("font/") ||
    value === "application/pdf" ||
    value === "application/zip" ||
    value === "application/gzip"
  );
}

// Sniff raw bytes for binary content: a NUL byte, or bytes that aren't valid
// UTF-8, mean "don't show as text". Only a bounded prefix is inspected for the
// NUL check; the full buffer is validated as UTF-8 so a truncated multi-byte
// sequence at the sample edge can't produce a false "text" result.
function looksBinary(bytes) {
  const sample = bytes.subarray(0, Math.min(bytes.length, 8192));
  if (sample.indexOf(0) !== -1) return true;
  try {
    new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    return false;
  } catch {
    return true;
  }
}

function contentUrl(mount, relativePath) {
  const url = new URL(`${FS_BASE}/content`, window.location.origin);
  url.searchParams.set("mount", mount);
  url.searchParams.set("path", relativePath);
  return url.pathname + url.search;
}

// List the mounts the viewer can browse, as `{ mount, label }`.
export async function listFsMounts() {
  const response = await apiFetch(`${FS_BASE}/mounts`);
  return response?.mounts || [];
}

// List a directory. An empty qualified path lists the mounts themselves; every
// returned entry's `path` is qualified so the tree can recurse with it directly.
export async function listWorkspace(qualifiedPath = "") {
  if (!qualifiedPath) {
    // Keep the backend area id in the query cache. Presentation components
    // translate known areas at render time so changing languages updates the
    // tree immediately without refetching mount data.
    const mounts = await listFsMounts();
    return {
      entries: mounts.map((mount) => ({
        name: mount.mount,
        path: mount.mount,
        is_dir: true,
      })),
    };
  }

  const { mount, path } = splitQualified(qualifiedPath);
  const url = new URL(`${FS_BASE}/list`, window.location.origin);
  url.searchParams.set("mount", mount);
  if (path) url.searchParams.set("path", path);
  const response = await apiFetch(url.pathname + url.search);
  const entries = (response?.entries || []).map((entry) => ({
    name: entry.name,
    path: joinQualified(mount, entry.path),
    is_dir: entry.kind === "directory",
  }));
  return { entries };
}

// Read a file for preview. Returns a discriminated shape the viewer renders:
// `{ kind: "text", content, ... }`, `{ kind: "image", image_data_url, ... }`,
// `{ kind: "binary", download_path, ... }`, or `{ kind: "directory" }`.
export async function readWorkspaceFile(qualifiedPath) {
  const { mount, path } = splitQualified(qualifiedPath);
  if (!mount || !path) {
    // A mount root is a directory, not a previewable file.
    return { kind: "directory", path: qualifiedPath };
  }

  const statUrl = new URL(`${FS_BASE}/stat`, window.location.origin);
  statUrl.searchParams.set("mount", mount);
  statUrl.searchParams.set("path", path);
  const statResponse = await apiFetch(statUrl.pathname + statUrl.search);
  const stat = statResponse?.stat || {};
  const mime = stat.mime_type || "application/octet-stream";
  const sizeBytes = Number(stat.size_bytes || 0);
  const download = contentUrl(mount, path);
  const base = { path: qualifiedPath, mime, size_bytes: sizeBytes, download_path: download };

  if (stat.kind && stat.kind !== "file") {
    return { ...base, kind: "directory" };
  }

  if (isImageMime(mime)) {
    // Gate by size before fetching/base64-expanding: an oversized image is
    // offered as a download rather than inlined into memory.
    if (sizeBytes > MAX_INLINE_IMAGE_BYTES) {
      return { ...base, kind: "binary" };
    }
    const image_data_url = await fetchAttachmentDataUrl(download);
    return { ...base, kind: "image", image_data_url };
  }

  // Too large to inline, or a known-binary type → offer a download without
  // fetching the bytes.
  if (isLikelyBinaryMime(mime) || sizeBytes > MAX_INLINE_TEXT_BYTES) {
    return { ...base, kind: "binary" };
  }

  // Otherwise fetch the bytes once and decide by content, not extension: a
  // text-like mime is trusted as text, and anything else (notably
  // `application/octet-stream` from an unknown extension like
  // `Dockerfile.worker`) is sniffed so real text still previews. Read through
  // the authed blob path (not apiFetch) so JSON/text bodies aren't auto-parsed.
  const blob = await fetchAttachmentBlob(download);
  const bytes = new Uint8Array(await blob.arrayBuffer());
  if (!isTextLikeMime(mime) && looksBinary(bytes)) {
    return { ...base, kind: "binary" };
  }
  const content = new TextDecoder("utf-8").decode(bytes);
  return { ...base, kind: "text", content };
}
