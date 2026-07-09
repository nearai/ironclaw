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
import { areaDisplayName } from "./workspace-presenters";

const FS_BASE = "/api/webchat/v2/fs";
const WORKSPACE_MOUNT = "workspace";
const MEMORY_MOUNT = "memory";

type WorkspaceCurrentUser =
  | {
      tenant_id?: string | null;
      user_id?: string | null;
    }
  | null
  | undefined;

type WorkspaceOptions = {
  currentUser?: WorkspaceCurrentUser;
  requireScopedWorkspace?: boolean;
};

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

function joinRelative(base, relativePath) {
  const basePath = String(base || "").replace(/^\/+|\/+$/g, "");
  const relative = String(relativePath || "").replace(/^\/+|\/+$/g, "");
  if (!basePath) return relative;
  return relative ? `${basePath}/${relative}` : basePath;
}

function stripRelativePrefix(path, prefix) {
  const value = String(path || "").replace(/^\/+/, "");
  const base = String(prefix || "").replace(/^\/+|\/+$/g, "");
  if (!base) return value;
  if (value === base) return "";
  return value.startsWith(`${base}/`) ? value.slice(base.length + 1) : value;
}

function userScopedPrefix(currentUser: WorkspaceCurrentUser) {
  const tenantId = String(currentUser?.tenant_id || "").replace(/^\/+|\/+$/g, "");
  const userId = String(currentUser?.user_id || "").replace(/^\/+|\/+$/g, "");
  if (!tenantId || !userId) return "";
  return `tenants/${tenantId}/users/${userId}`;
}

function emptyDirectoryResponse() {
  return { entries: [] };
}

function isDirectoryEntry(entry) {
  return entry?.kind === "directory";
}

function hasDirectoryNamed(response, name) {
  return (response?.entries || []).some(
    (entry) => entry.name === name && isDirectoryEntry(entry)
  );
}

function isMemorySidecarEntry(entry) {
  const name = String(entry?.name || "");
  return (
    name.endsWith(".meta") ||
    name.endsWith(".chunks") ||
    name.endsWith(".versions")
  );
}

function memoryVisibleEntries(response) {
  return (response?.entries || []).filter((entry) => !isMemorySidecarEntry(entry));
}

function soleVisibleDirectory(response) {
  const entries = memoryVisibleEntries(response);
  if (entries.length !== 1 || !isDirectoryEntry(entries[0])) return null;
  return entries[0];
}

function splitRelative(relativePath) {
  return String(relativePath || "")
    .split("/")
    .filter(Boolean);
}

function isNotFound(error) {
  return error?.status === 404;
}

function fsListUrl(mount, relativePath) {
  const url = new URL(`${FS_BASE}/list`, window.location.origin);
  url.searchParams.set("mount", mount);
  if (relativePath) url.searchParams.set("path", relativePath);
  return url.pathname + url.search;
}

async function fetchFsList(mount, relativePath) {
  return apiFetch(fsListUrl(mount, relativePath));
}

// Hosted WebUI storage exposes caller files below a tenant/user subtree. Local
// single-user workspaces usually do not, so raw-root fallback is allowed only
// when the deployment did not require scoped workspace projection and the raw
// root does not look like the hosted multi-user container.
async function resolveWorkspaceRootWithOptions(
  currentUser: WorkspaceCurrentUser,
  requireScopedWorkspace: boolean,
) {
  const prefix = userScopedPrefix(currentUser);
  if (!prefix) return { prefix: "", rootResponse: null };
  try {
    const rootResponse = await fetchFsList(WORKSPACE_MOUNT, prefix);
    return { prefix, rootResponse };
  } catch (error) {
    if (!isNotFound(error)) throw error;
    const rootResponse = await fetchFsList(WORKSPACE_MOUNT, "");
    if (requireScopedWorkspace || hasDirectoryNamed(rootResponse, "tenants")) {
      return { prefix, rootResponse: emptyDirectoryResponse() };
    }
    return { prefix: "", rootResponse };
  }
}

async function resolveWorkspaceDirectory(
  relativePath,
  currentUser: WorkspaceCurrentUser,
  requireScopedWorkspace: boolean,
) {
  const resolved = await resolveWorkspaceRootWithOptions(
    currentUser,
    requireScopedWorkspace,
  );
  if (resolved.prefix) {
    const actualPath = joinRelative(resolved.prefix, relativePath);
    if (!relativePath) return { actualPath, response: resolved.rootResponse };
    return { actualPath, response: await fetchFsList(WORKSPACE_MOUNT, actualPath) };
  }
  if (!relativePath && resolved.rootResponse) {
    return { actualPath: "", response: resolved.rootResponse };
  }
  return { actualPath: relativePath, response: await fetchFsList(WORKSPACE_MOUNT, relativePath) };
}

async function resolveWorkspacePath(
  relativePath,
  currentUser: WorkspaceCurrentUser,
  requireScopedWorkspace: boolean,
) {
  const { prefix } = await resolveWorkspaceRootWithOptions(
    currentUser,
    requireScopedWorkspace,
  );
  return prefix ? joinRelative(prefix, relativePath) : relativePath;
}

function shouldCollapseMemoryDirectory(actualPath, directoryName) {
  return (
    directoryName === "agents" ||
    directoryName === "projects" ||
    actualPath.endsWith("/agents") ||
    actualPath.endsWith("/projects")
  );
}

async function collapseMemoryDirectory(actualPath, response) {
  let nextPath = actualPath;
  let nextResponse = response;

  for (let i = 0; i < 8; i += 1) {
    const directory = soleVisibleDirectory(nextResponse);
    if (!directory || !shouldCollapseMemoryDirectory(nextPath, directory.name)) break;
    nextPath = joinRelative(nextPath, directory.name);
    nextResponse = await fetchFsList(MEMORY_MOUNT, nextPath);
  }

  return { actualPath: nextPath, response: nextResponse };
}

async function resolveMemoryDirectory(relativePath, currentUser: WorkspaceCurrentUser) {
  const prefix = userScopedPrefix(currentUser);
  if (!prefix) {
    return { actualPath: relativePath, response: await fetchFsList(MEMORY_MOUNT, relativePath) };
  }

  let actualPath = prefix;
  let response;
  try {
    response = await fetchFsList(MEMORY_MOUNT, actualPath);
  } catch (error) {
    if (!isNotFound(error)) throw error;
    response = emptyDirectoryResponse();
  }

  let resolved = await collapseMemoryDirectory(actualPath, response);
  actualPath = resolved.actualPath;
  response = resolved.response;

  for (const segment of splitRelative(relativePath)) {
    actualPath = joinRelative(actualPath, segment);
    response = await fetchFsList(MEMORY_MOUNT, actualPath);
    resolved = await collapseMemoryDirectory(actualPath, response);
    actualPath = resolved.actualPath;
    response = resolved.response;
  }

  return { actualPath, response };
}

async function resolveMemoryPath(relativePath, currentUser: WorkspaceCurrentUser) {
  const segments = splitRelative(relativePath);
  const basename = segments.pop();
  if (!basename || !userScopedPrefix(currentUser)) return relativePath;
  const { actualPath } = await resolveMemoryDirectory(segments.join("/"), currentUser);
  return joinRelative(actualPath, basename);
}

async function resolveDirectory(
  mount,
  path,
  { currentUser, requireScopedWorkspace = false }: WorkspaceOptions,
) {
  if (mount === WORKSPACE_MOUNT) {
    return resolveWorkspaceDirectory(path, currentUser, requireScopedWorkspace);
  }
  if (mount === MEMORY_MOUNT) {
    return resolveMemoryDirectory(path, currentUser);
  }
  return { actualPath: path, response: await fetchFsList(mount, path) };
}

async function resolveFilePath(
  mount,
  path,
  { currentUser, requireScopedWorkspace = false }: WorkspaceOptions,
) {
  if (mount === WORKSPACE_MOUNT) {
    return resolveWorkspacePath(path, currentUser, requireScopedWorkspace);
  }
  if (mount === MEMORY_MOUNT) {
    return resolveMemoryPath(path, currentUser);
  }
  return path;
}

function visibleResponseEntries(mount, response) {
  if (mount === MEMORY_MOUNT) return memoryVisibleEntries(response);
  return response?.entries || [];
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
export async function listWorkspace(
  qualifiedPath = "",
  { currentUser, requireScopedWorkspace = false }: WorkspaceOptions = {},
) {
  if (!qualifiedPath) {
    // The root lists the storage areas as plain top-level folders (memory,
    // home) — the "mount" concept is never surfaced in the UI. `path` is the
    // backend area id (used to route reads); `name` is the friendly display
    // name, so navigation stays unambiguous without exposing area ids.
    const mounts = await listFsMounts();
    return {
      entries: mounts.map((mount) => ({
        name: areaDisplayName(mount.mount),
        path: mount.mount,
        is_dir: true,
      })),
    };
  }

  const { mount, path } = splitQualified(qualifiedPath);
  const { actualPath, response } = await resolveDirectory(mount, path, {
    currentUser,
    requireScopedWorkspace,
  });
  const entries = visibleResponseEntries(mount, response).map((entry) => ({
    name: entry.name,
    path: joinQualified(mount, joinRelative(path, stripRelativePrefix(entry.path, actualPath))),
    is_dir: entry.kind === "directory",
  }));
  return { entries };
}

// Read a file for preview. Returns a discriminated shape the viewer renders:
// `{ kind: "text", content, ... }`, `{ kind: "image", image_data_url, ... }`,
// `{ kind: "binary", download_path, ... }`, or `{ kind: "directory" }`.
export async function readWorkspaceFile(
  qualifiedPath,
  { currentUser, requireScopedWorkspace = false }: WorkspaceOptions = {},
) {
  const { mount, path } = splitQualified(qualifiedPath);
  if (!mount || !path) {
    // A mount root is a directory, not a previewable file.
    return { kind: "directory", path: qualifiedPath };
  }

  const actualPath = await resolveFilePath(mount, path, {
    currentUser,
    requireScopedWorkspace,
  });
  const statUrl = new URL(`${FS_BASE}/stat`, window.location.origin);
  statUrl.searchParams.set("mount", mount);
  statUrl.searchParams.set("path", actualPath);
  const statResponse = await apiFetch(statUrl.pathname + statUrl.search);
  const stat = statResponse?.stat || {};
  const mime = stat.mime_type || "application/octet-stream";
  const sizeBytes = Number(stat.size_bytes || 0);
  const download = contentUrl(mount, actualPath);
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
