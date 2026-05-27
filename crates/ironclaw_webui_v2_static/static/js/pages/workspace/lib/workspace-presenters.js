export const DEFAULT_WORKSPACE_PATH = "README.md";

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
