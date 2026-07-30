// Shared recognition for assistant-authored links to thread-scoped workspace
// files. Keep this deliberately narrower than a general URL parser: only the
// scoped file paths already surfaced by project-file chips are eligible for the
// authenticated in-app preview.

export const WORKSPACE_FILE_PATH_SOURCE =
  String.raw`\/workspace\/[A-Za-z0-9._\-/]+\.[A-Za-z0-9]+`;

const EXACT_WORKSPACE_FILE_PATH = new RegExp(`^${WORKSPACE_FILE_PATH_SOURCE}$`);
const SANDBOX_SCHEME = /^sandbox:/i;

export function workspaceFilePathFromHref(href: unknown): string | null {
  if (typeof href !== "string") return null;
  const trimmed = href.trim();
  const path = SANDBOX_SCHEME.test(trimmed)
    ? trimmed.replace(SANDBOX_SCHEME, "")
    : trimmed;

  if (!EXACT_WORKSPACE_FILE_PATH.test(path)) return null;
  const segments = path.slice("/workspace/".length).split("/");
  if (
    segments.some(
      (segment) => !segment || segment === "." || segment === "..",
    )
  ) {
    return null;
  }
  return path;
}
