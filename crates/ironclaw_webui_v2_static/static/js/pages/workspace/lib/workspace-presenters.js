// Empty selection = the viewer's root, where the tree lists the storage areas.
// The browser then drills in by area.
export const DEFAULT_WORKSPACE_PATH = "";

// Display names for the storage areas shown at the root. Internally the first
// path segment is the backend area id (used for routing and the URL); the UI
// renders these friendlier names instead — the local project directory shows
// as "home", memory stays "memory".
export const AREA_DISPLAY_NAMES = { workspace: "home" };

export function areaDisplayName(areaId) {
  return AREA_DISPLAY_NAMES[areaId] || areaId;
}

// Canonical entry ordering, applied in every panel so the tree and the main
// listing never disagree: directories first, then files, each group sorted
// alphabetically (case-insensitive, locale-aware) by display name.
export function sortEntries(entries) {
  return [...(entries || [])].sort((a, b) => {
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
    return a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
  });
}

export function pathSegments(path) {
  if (!path) return [];
  return path.split("/").filter(Boolean);
}

export function routeForWorkspacePath(path) {
  if (!path) return "/workspace";
  return `/workspace/${pathSegments(path).map(encodeURIComponent).join("/")}`;
}

export function parentPath(path) {
  const parts = pathSegments(path);
  parts.pop();
  return parts.join("/");
}

export function isMarkdownPath(path) {
  return /\.mdx?$/i.test(path || "");
}

export function formatWorkspaceDate(iso) {
  if (!iso) return "Not indexed";
  return new Date(iso).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function snippetFor(text, query, length = 140) {
  const content = String(text || "");
  const needle = String(query || "").trim().toLowerCase();
  if (!needle) return content.slice(0, length);
  const index = content.toLowerCase().indexOf(needle);
  if (index < 0) return content.slice(0, length);
  const start = Math.max(0, index - Math.floor(length / 2));
  const end = Math.min(content.length, start + length);
  return `${start > 0 ? "..." : ""}${content.slice(start, end)}${end < content.length ? "..." : ""}`;
}
