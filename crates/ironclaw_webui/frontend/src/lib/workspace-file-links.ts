// Shared recognition for assistant-authored links to thread-scoped workspace
// files. This is deliberately narrower than a general URL parser: workspace
// paths have no query/fragment, decode exactly once, and stay below the scoped
// root after decoding.

const WORKSPACE_PREFIX = "/workspace/";
const SANDBOX_SCHEME = /^sandbox:/i;
// Reject encoded percent signs as well as separators. A literal `%25` could
// otherwise become another escape sequence if any downstream layer decoded the
// already-validated path again.
const AMBIGUOUS_PATH_ENCODING = /%(?:25|2f|5c)/i;
const CONTROL_CHARACTER = /[\u0000-\u001f\u007f]/;

function isSafeDecodedWorkspacePath(path: string): boolean {
  if (!path.startsWith(WORKSPACE_PREFIX)) return false;
  const segments = path.slice(WORKSPACE_PREFIX.length).split("/");
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
  if (typeof href !== "string") return null;
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
    AMBIGUOUS_PATH_ENCODING.test(encodedPath)
  ) {
    return null;
  }

  let decodedPath: string;
  try {
    decodedPath = decodeURIComponent(encodedPath);
  } catch {
    return null;
  }
  return isSafeDecodedWorkspacePath(decodedPath) ? decodedPath : null;
}

export function workspaceFileHrefFromPath(path: unknown): string | null {
  if (!isValidWorkspaceFilePath(path)) return null;
  const segments = path.slice(WORKSPACE_PREFIX.length).split("/");
  return `${WORKSPACE_PREFIX}${segments.map(encodeURIComponent).join("/")}`;
}

export function workspaceViewerRouteFromFilePath(
  path: unknown,
): string | null {
  const fileHref = workspaceFileHrefFromPath(path);
  return fileHref ? `/workspace${fileHref}` : null;
}
