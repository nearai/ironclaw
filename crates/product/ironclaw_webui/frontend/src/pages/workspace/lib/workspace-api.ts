// Read-only filesystem-viewer API client.
//
// Wraps the WebChat v2 `/fs/*` endpoints (backed by the Reborn
// `FilesystemBrowseReader` port) and the source-thread `/threads/*/files`
// endpoints as the path-oriented surface the workspace tree/viewer consume.
// A "qualified path" used throughout the UI is
// `"<mount>/<mount-relative-path>"` — the first segment selects the mount
// (memory/workspace/…), the rest is the path within it. The empty qualified
// path is the root, which lists the available mounts as top-level directories,
// so the tree itself doubles as the mount picker. Strictly read-only: there is
// no write/save path here.

import {
  apiFetch,
  fetchAttachmentBlob,
  fetchAttachmentDataUrl,
  listProjectFiles,
  projectFileContentUrl,
  statProjectFile,
} from "../../../lib/api";
import { isValidWorkspaceFilePath } from "../../../lib/workspace-file-links";

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
  threadId?: string | null;
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

// `/fs/*` paths are mount-relative; the server binds each call to the
// authenticated caller and returns only that caller's files. The frontend
// never builds tenant/user storage prefixes: `currentUser` is used only as a
// fail-closed gate when the deployment requires a scoped workspace projection
// (hosted/multi-user). When a scoped projection is required and the caller
// identity is unavailable, listings render empty and file reads short-circuit
// to a directory placeholder, so an unauthenticated or identity-still-loading
// session can never request a raw mount path.
// `currentUser` is a fail-closed identity gate, not a path component: the
// `/fs/*` handlers resolve the caller's scoped mount server-side. This predicate
// only checks identity presence; it never formats a tenant/user path, so no
// caller can re-introduce the double-prefix regression.
function hasCallerIdentity(currentUser: WorkspaceCurrentUser) {
  return Boolean(currentUser?.tenant_id && currentUser?.user_id);
}

function scopedUserUnavailable(
  currentUser: WorkspaceCurrentUser,
  requireScopedWorkspace: boolean,
) {
  return requireScopedWorkspace && !hasCallerIdentity(currentUser);
}

function emptyDirectoryResponse() {
  return { entries: [] };
}

function isDirectoryEntry(entry) {
  return entry?.kind === "directory";
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

// `/fs/*` paths are mount-relative and caller-scoped server-side. The
// frontend only needs to: (1) fail closed when a scoped projection is
// required but the caller identity is unavailable, and (2) pass the
// selected mount-relative path through. No tenant/user prefix is ever
// prepended — the server is the authority for caller scope.
async function resolveWorkspaceDirectory(
  relativePath,
  currentUser: WorkspaceCurrentUser,
  requireScopedWorkspace: boolean,
) {
  if (scopedUserUnavailable(currentUser, requireScopedWorkspace)) {
    return { actualPath: "", response: emptyDirectoryResponse() };
  }
  return {
    actualPath: relativePath,
    response: await fetchFsList(WORKSPACE_MOUNT, relativePath),
  };
}

async function resolveWorkspacePath(
  relativePath,
  currentUser: WorkspaceCurrentUser,
  requireScopedWorkspace: boolean,
) {
  if (scopedUserUnavailable(currentUser, requireScopedWorkspace)) return "";
  return relativePath;
}

function shouldCollapseMemoryDirectory(actualPath, directoryName) {
  return (
    directoryName === "agents" ||
    directoryName === "projects" ||
    actualPath === "agents" ||
    actualPath === "projects" ||
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

// Memory is caller-scoped server-side: `/fs/list?mount=memory` at the root
// returns the authenticated caller's memory document tree. The frontend
// collapses the internal `agents/<agent>/projects/_none` wrapper directories
// so the user-facing root jumps straight to the caller's memory content, but
// every request stays mount-relative.
async function resolveMemoryDirectory(
  relativePath,
  currentUser: WorkspaceCurrentUser,
  requireScopedWorkspace: boolean,
) {
  if (scopedUserUnavailable(currentUser, requireScopedWorkspace)) {
    return { actualPath: "", response: emptyDirectoryResponse() };
  }

  // Seed the walk at the mount root and let the segment loop descend through
  // `relativePath` exactly once. Seeding with `relativePath` would list it once
  // up front and then append every segment again, doubling any non-root path.
  let actualPath = "";
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

// Resolve a memory file's mount-relative path by walking its parent directory
// through the collapse-walk (which transparently skips the internal
// `agents/<agent>/projects/_none` wrapper directories the server returns for
// the caller's own memory tree), then joining the basename. Every request
// stays mount-relative; the server is the authority for caller scope.
async function resolveMemoryPath(
  relativePath,
  currentUser: WorkspaceCurrentUser,
  requireScopedWorkspace: boolean,
) {
  const segments = splitRelative(relativePath);
  const basename = segments.pop();
  if (scopedUserUnavailable(currentUser, requireScopedWorkspace)) return "";
  if (!basename) return relativePath;
  const { actualPath } = await resolveMemoryDirectory(
    segments.join("/"),
    currentUser,
    requireScopedWorkspace,
  );
  return joinRelative(actualPath, basename);
}

async function resolveDirectory(
  mount,
  path,
  { currentUser, requireScopedWorkspace = true }: WorkspaceOptions,
) {
  if (mount === WORKSPACE_MOUNT) {
    return resolveWorkspaceDirectory(path, currentUser, requireScopedWorkspace);
  }
  if (mount === MEMORY_MOUNT) {
    return resolveMemoryDirectory(path, currentUser, requireScopedWorkspace);
  }
  return { actualPath: path, response: await fetchFsList(mount, path) };
}

async function resolveFilePath(
  mount,
  path,
  { currentUser, requireScopedWorkspace = true }: WorkspaceOptions,
) {
  if (mount === WORKSPACE_MOUNT) {
    return resolveWorkspacePath(path, currentUser, requireScopedWorkspace);
  }
  if (mount === MEMORY_MOUNT) {
    return resolveMemoryPath(path, currentUser, requireScopedWorkspace);
  }
  return path;
}

function visibleResponseEntries(mount, response) {
  if (mount === MEMORY_MOUNT) return memoryVisibleEntries(response);
  return response?.entries || [];
}

function projectPathFromQualified(qualifiedPath) {
  const { mount, path } = splitQualified(qualifiedPath);
  if (mount !== "workspace") {
    throw new Error("Thread-scoped browsing is limited to the workspace mount");
  }
  const projectPath = path ? `/workspace/${path}` : "/workspace";
  if (projectPath !== "/workspace" && !isValidWorkspaceFilePath(projectPath)) {
    throw new Error("Invalid thread-scoped workspace path");
  }
  return projectPath;
}

function qualifiedProjectPath(path) {
  const normalized = String(path || "").replace(/^\/+/, "");
  const qualified = normalized.startsWith("workspace/") || normalized === "workspace"
    ? normalized
    : `workspace/${normalized}`;
  if (
    qualified !== "workspace" &&
    !isValidWorkspaceFilePath(`/${qualified}`)
  ) {
    return null;
  }
  return qualified;
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
  {
    currentUser,
    requireScopedWorkspace = true,
    threadId,
  }: WorkspaceOptions = {},
) {
  if (threadId) {
    if (!qualifiedPath) {
      return {
        entries: [{ name: "workspace", path: "workspace", is_dir: true }],
      };
    }
    const response = await listProjectFiles({
      threadId,
      path: projectPathFromQualified(qualifiedPath),
    });
    return {
      entries: (response?.entries || []).flatMap((entry) => {
        const path = qualifiedProjectPath(entry.path);
        return path
          ? [{ name: entry.name, path, is_dir: entry.kind === "directory" }]
          : [];
      }),
    };
  }
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
  {
    currentUser,
    requireScopedWorkspace = true,
    threadId,
  }: WorkspaceOptions = {},
) {
  const { mount, path } = splitQualified(qualifiedPath);
  if (!mount || !path) {
    // A mount root is a directory, not a previewable file.
    return { kind: "directory", path: qualifiedPath };
  }
  if (
    !threadId &&
    (mount === WORKSPACE_MOUNT || mount === MEMORY_MOUNT) &&
    scopedUserUnavailable(currentUser, requireScopedWorkspace)
  ) {
    return { kind: "directory", path: qualifiedPath };
  }

  let statResponse;
  let download;
  if (threadId) {
    const projectPath = projectPathFromQualified(qualifiedPath);
    statResponse = await statProjectFile({ threadId, path: projectPath });
    download = projectFileContentUrl({ threadId, path: projectPath });
  } else {
    const actualPath = await resolveFilePath(mount, path, {
      currentUser,
      requireScopedWorkspace,
    });
    const statUrl = new URL(`${FS_BASE}/stat`, window.location.origin);
    statUrl.searchParams.set("mount", mount);
    statUrl.searchParams.set("path", actualPath);
    statResponse = await apiFetch(statUrl.pathname + statUrl.search);
    download = contentUrl(mount, actualPath);
  }
  const stat = statResponse?.stat || {};
  const mime = stat.mime_type || "application/octet-stream";
  const sizeBytes = Number(stat.size_bytes || 0);
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
