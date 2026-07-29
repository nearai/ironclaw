import { Card, Skeleton } from "@ironclaw/ui";
import { useT } from "../../../lib/i18n";
import { areaDisplayName, sortEntries } from "../lib/workspace-presenters";
import { WorkspaceBreadcrumb } from "./workspace-breadcrumb";

// Files/dirs whose any path segment starts with "." stay hidden in the UI
// (engine/system internals); their bytes are denied server-side regardless.
function isUiHiddenWorkspacePath(path = "") {
  return String(path)
    .split("/")
    .some((segment) => segment.startsWith("."));
}

// Main-pane directory listing (master→detail): selecting a folder in the tree
// or breadcrumb shows its contents here as a clickable list, instead of forcing
// the user to drill the tree alone. Clicking an entry navigates to it (a folder
// opens here; a file opens in the viewer).
export function WorkspaceDirectory({ path, entries, isLoading, filter, onOpen, onNavigate }) {
  const t = useT();

  if (isLoading) {
    return (
      <div className="space-y-4">
        <Skeleton className="h-16" />
        <Skeleton className="h-[460px]" />
      </div>
    );
  }

  const visible = (entries || []).filter((entry) => !isUiHiddenWorkspacePath(entry.path));
  const displayName = (entry) => path ? entry.name : areaDisplayName(entry.path, t);
  const needle = String(filter || "").trim().toLowerCase();
  const filtered = needle
    ? visible.filter((entry) => displayName(entry).toLowerCase().includes(needle))
    : visible;
  const rows = sortEntries(filtered, displayName);

  let body;
  if (!visible.length) {
    body = (<div className="px-4 py-10 text-center text-sm text-[var(--v2-text-muted)]">{t("workspace.emptyDir")}</div>);
  } else if (!rows.length) {
    body = (<div className="px-4 py-10 text-center text-sm text-[var(--v2-text-muted)]">{t("workspace.noMatches")}</div>);
  } else {
    // The row rules keep main's `white/[0.06]` rather than the panel-border
    // token: on the light card that value is near-invisible, and the token
    // hairline reads as an extra line the page never had. This extraction is
    // meant to be visually inert, so parity wins over tidiness here.
    body = (
      <div className="divide-y divide-white/[0.06]">
        {rows.map((entry) => (
          <button
            key={entry.path}
            type="button"
            data-testid="workspace-directory-entry"
            data-entry-path={entry.path}
            onClick={() => onOpen(entry.path)}
            className="flex w-full items-center gap-3 px-4 py-2.5 text-left text-sm text-[var(--v2-text-strong)] transition-colors hover:bg-[var(--v2-surface-soft)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--v2-focus-ring)]"
          >
            <span className={["w-4 text-center text-xs", entry.is_dir ? "text-[var(--v2-accent-text)]" : "text-[var(--v2-text-muted)]"].join(" ")}>
              {entry.is_dir ? "□" : "·"}
            </span>
            <span className={["min-w-0 truncate", entry.is_dir ? "font-semibold" : ""].join(" ")}>{displayName(entry)}</span>
          </button>
        ))}
      </div>
    );
  }

  return (
    <Card className="flex min-h-[520px] flex-col overflow-hidden xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--v2-panel-border)] px-4 py-3">
        <WorkspaceBreadcrumb path={path} onNavigate={onNavigate} />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">{body}</div>
    </Card>
  );
}
