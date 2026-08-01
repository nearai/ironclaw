// Shared recognition for assistant-authored links to thread-scoped workspace
// files. This is deliberately narrower than a general URL parser: workspace
// paths have no query/fragment, decode exactly once, and stay below the scoped
// root after decoding.

const WORKSPACE_PREFIX = "/workspace/";
const MAX_RAW_WORKSPACE_HREF_LENGTH = 8_192;
const MAX_DECODED_WORKSPACE_PATH_LENGTH = 4_096;
const MAX_WORKSPACE_PATH_SEGMENTS = 64;
const SANDBOX_SCHEME = /^sandbox:/i;
const ENCODED_SEPARATOR = /%(?:2f|5c)/i;
// After the one supported decode, no structural escape may remain for another
// layer to reinterpret. Ordinary literal percent signs remain valid.
const REMAINING_STRUCTURAL_ESCAPE = /%(?:2e|2f|5c)/i;
const CONTROL_CHARACTER = /[\u0000-\u001f\u007f]/;

function isSafeDecodedWorkspacePath(path: string): boolean {
  if (
    path.length > MAX_DECODED_WORKSPACE_PATH_LENGTH ||
    !path.startsWith(WORKSPACE_PREFIX)
  ) {
    return false;
  }
  const segments = path.slice(WORKSPACE_PREFIX.length).split("/");
  if (segments.length > MAX_WORKSPACE_PATH_SEGMENTS) return false;
  return !segments.some(
    (segment) =>
      !segment ||
      segment === "." ||
      segment === ".." ||
      segment.includes("\\") ||
      CONTROL_CHARACTER.test(segment),
  );
}

export function isValidWorkspaceFilePath(path: unknown): path is string {
  return typeof path === "string" && isSafeDecodedWorkspacePath(path);
}

export function workspaceFilePathFromHref(href: unknown): string | null {
  if (
    typeof href !== "string" ||
    href.length > MAX_RAW_WORKSPACE_HREF_LENGTH
  ) {
    return null;
  }
  const trimmed = href.trim();
  const encodedPath = SANDBOX_SCHEME.test(trimmed)
    ? trimmed.replace(SANDBOX_SCHEME, "")
    : trimmed;

  // Reject URL structure and encoded separators before decoding so a filename
  // cannot be reinterpreted as a query, fragment, or additional path segment.
  if (
    !encodedPath.startsWith(WORKSPACE_PREFIX) ||
    encodedPath.includes("?") ||
    encodedPath.includes("#") ||
    ENCODED_SEPARATOR.test(encodedPath)
  ) {
    return null;
  }

  let decodedPath: string;
  try {
    decodedPath = decodeURIComponent(encodedPath);
  } catch {
    return null;
  }
  if (REMAINING_STRUCTURAL_ESCAPE.test(decodedPath)) return null;
  return isSafeDecodedWorkspacePath(decodedPath) ? decodedPath : null;
}

export function workspaceFileHrefFromPath(path: unknown): string | null {
  if (!isValidWorkspaceFilePath(path)) return null;
  const segments = path.slice(WORKSPACE_PREFIX.length).split("/");
  const href =
    `${WORKSPACE_PREFIX}${segments.map(encodeURIComponent).join("/")}`;
  return href.length <= MAX_RAW_WORKSPACE_HREF_LENGTH ? href : null;
}

export function workspaceViewerRouteFromFilePath(
  path: unknown,
  projectId?: unknown,
): string | null {
  const fileHref = workspaceFileHrefFromPath(path);
  if (!fileHref) return null;
  const route = `/workspace${fileHref}`;
  if (typeof projectId !== "string" || !projectId) return route;
  return `${route}?project_id=${encodeURIComponent(projectId)}`;
}
