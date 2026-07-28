// Pure helpers for surfacing agent-referenced workspace files as downloads.
//
// Kept free of browser/React imports so the path-extraction logic — which gates
// the download side effect — is unit-testable on its own.

// Match scoped workspace paths with a file extension. These are the paths the
// `/files/content` endpoint serves and that the agent's file tools emit. Both a
// bare mention (`/workspace/report.csv`) and a markdown link href
// (`[report.csv](/workspace/report.csv)`) are caught by the same scan.
const WORKSPACE_FILE_PATH = /\/workspace\/[A-Za-z0-9._\-/]+\.[A-Za-z0-9]+/g;

// Strip fenced (```…```) and inline (`…`) code spans. A path that the assistant
// only *displays* in a shell snippet (`cat /workspace/.env`) must not become a
// real, one-click download chip — code spans are illustrative, not file
// references. Fenced blocks are removed first so their inner backticks don't
// throw off the inline pass.
function stripCodeSpans(content) {
  return content.replace(/```[\s\S]*?```/g, " ").replace(/`[^`]*`/g, " ");
}

// Extract de-duplicated workspace file paths, preserving first-seen order.
export function extractWorkspaceFilePaths(content) {
  if (typeof content !== "string" || !content) return [];
  const seen = new Set();
  const paths = [];
  for (const match of stripCodeSpans(content).matchAll(WORKSPACE_FILE_PATH)) {
    // The regex ends at `\.[A-Za-z0-9]+`, so a match always terminates on an
    // alphanumeric character — trailing sentence/link punctuation (`. , ) ]`)
    // is never captured and needs no stripping here.
    const path = match[0];
    if (!seen.has(path)) {
      seen.add(path);
      paths.push(path);
    }
  }
  return paths;
}

export function basename(path) {
  return path.split("/").filter(Boolean).pop() || path;
}

export function formatSize(bytes) {
  if (typeof bytes !== "number" || !Number.isFinite(bytes)) return "";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}
