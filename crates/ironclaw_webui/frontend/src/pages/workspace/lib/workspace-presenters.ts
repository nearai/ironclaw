// Empty selection = the viewer's root, where the tree lists the storage areas.
// The browser then drills in by area.
export const DEFAULT_WORKSPACE_PATH = "";

// Translation keys for the storage areas shown at the root. Internally the
// first path segment remains the backend area id used for routing and URLs.
// Unknown future areas deliberately fall back to that id until their owning
// feature adds a user-facing label.
export const AREA_DISPLAY_KEYS = {
  workspace: "workspace.area.home",
  memory: "workspace.area.memory",
};

export function areaDisplayName(areaId, t) {
  const key = Object.hasOwn(AREA_DISPLAY_KEYS, areaId) ? AREA_DISPLAY_KEYS[areaId] : null;
  return key && typeof t === "function" ? t(key) : areaId;
}

// Format the binary byte counts returned by the filesystem API into compact,
// locale-aware labels. The product has historically used KB/MB wording for
// 1024-based thresholds, so retain that user-facing convention here.
export function formatWorkspaceFileSize(bytes, locale = "en") {
  if (bytes == null) return "";
  const size = Number(bytes);
  if (!Number.isFinite(size) || size < 0) return "";

  const units = ["byte", "kilobyte", "megabyte", "gigabyte", "terabyte", "petabyte"];
  let unitIndex = 0;
  let value = size;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }

  const maximumFractionDigits = value >= 10 || Number.isInteger(value) ? 0 : 1;
  const options: Intl.NumberFormatOptions = {
    style: "unit",
    unit: units[unitIndex],
    unitDisplay: unitIndex === 0 ? "long" : "short",
    maximumFractionDigits,
  };
  try {
    return new Intl.NumberFormat(locale || "en", options).format(value);
  } catch {
    try {
      return new Intl.NumberFormat("en", options).format(value);
    } catch {
      // Older browsers may not support every unit. Keep the render tree alive
      // with a dependency-free label even when the English retry also fails.
      const precision = 10 ** maximumFractionDigits;
      const rounded = Math.round(value * precision) / precision;
      const fallbackUnits = [size === 1 ? "byte" : "bytes", "KB", "MB", "GB", "TB", "PB"];
      return `${rounded} ${fallbackUnits[unitIndex]}`;
    }
  }
}

// Canonical entry ordering, applied in every panel so the tree and the main
// listing never disagree: directories first, then files, each group sorted
// alphabetically (case-insensitive, locale-aware) by display name.
export function sortEntries(entries, displayName = (entry) => entry.name) {
  return [...(entries || [])].sort((a, b) => {
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
    const left = String(displayName(a) ?? "");
    const right = String(displayName(b) ?? "");
    return left.localeCompare(right, undefined, { sensitivity: "base" });
  });
}

export function pathSegments(path) {
  if (!path) return [];
  return path.split("/").filter(Boolean);
}

// Expand every directory above the selected path while retaining branches the
// user opened manually. Returning the existing Set when nothing changes keeps
// route updates idempotent and avoids an unnecessary tree render.
export function expandWorkspaceSelection(
  expandedPaths: Set<string>,
  selectedPath: string | undefined,
): Set<string> {
  const parentSegments = pathSegments(selectedPath).slice(0, -1);
  const parentPaths = parentSegments.map((_, index) =>
    parentSegments.slice(0, index + 1).join("/"),
  );

  if (parentPaths.every((path) => expandedPaths.has(path))) {
    return expandedPaths;
  }

  return new Set([...expandedPaths, ...parentPaths]);
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
