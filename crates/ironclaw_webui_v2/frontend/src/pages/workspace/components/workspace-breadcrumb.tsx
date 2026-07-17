import { Button } from "../../../design-system/button";
import { useT } from "../../../lib/i18n";
import { areaDisplayName, pathSegments, routeForWorkspacePath } from "../lib/workspace-presenters";

// Path breadcrumb shared by the file viewer and the directory listing. The root
// is shown as the localized "workspace" label (both areas live under it); the
// first segment is a storage area, rendered by its display name ("home"/"memory")
// while still navigating by its real id. Every crumb uses the same URL-as-state
// path the tree uses, so breadcrumb clicks, tree clicks, and direct links stay
// in sync.
export function WorkspaceBreadcrumb({ path, onNavigate }) {
  const t = useT();
  const parts = pathSegments(path);
  let current = "";

  return (
    <div className="flex min-w-0 flex-wrap items-center gap-1 font-mono text-sm">
      <Button
        type="button"
        variant="ghost"
        size="sm"
        onClick={() => onNavigate("/workspace")}
        className="h-auto px-1.5 py-0.5 text-[var(--v2-accent-text)] hover:underline"
      >
        {t("workspace.breadcrumbRoot")}
      </Button>
      {parts.map((part, index) => {
        current = current ? `${current}/${part}` : part;
        const target = current;
        const label = index === 0 ? areaDisplayName(part, t) : part;
        return (
          <span key={target} className="inline-flex min-w-0 items-center gap-1">
            <span className="text-[var(--v2-text-faint)]">/</span>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => onNavigate(routeForWorkspacePath(target))}
              className="h-auto max-w-[220px] truncate px-1.5 py-0.5 text-[var(--v2-accent-text)] hover:underline"
            >
              {label}
            </Button>
          </span>
        );
      })}
    </div>
  );
}
