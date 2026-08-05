import { useT } from "../../../lib/i18n";
import { areaDisplayName, pathSegments } from "../lib/workspace-presenters";

// Path breadcrumb shared by the file viewer and the directory listing. The root
// is shown as the localized "workspace" label (both areas live under it); the
// first segment is a storage area, rendered by its display name ("home"/"memory")
// while still navigating by its real id. Every crumb emits the same qualified
// path the tree uses; the page then applies either its caller-default or
// thread-scoped route prefix, so clicks and direct links stay in sync.
export function WorkspaceBreadcrumb({ path, onNavigate }) {
  const t = useT();
  const parts = pathSegments(path);
  let current = "";

  return (
    <nav
      aria-label={t("workspace.breadcrumbRoot")}
      className="flex min-w-0 flex-wrap items-center gap-2 font-mono text-sm"
    >
      <button
        type="button"
        onClick={() => onNavigate("")}
        className="text-signal hover:underline"
      >
        {t("workspace.breadcrumbRoot")}
      </button>
      {parts.map((part, index) => {
        current = current ? `${current}/${part}` : part;
        const target = current;
        const label = index === 0 ? areaDisplayName(part, t) : part;
        return (
          <>
          <span key={target} className="text-iron-400">/</span>
          <button
            key={`${target}-button`}
            type="button"
            onClick={() => onNavigate(target)}
            className="max-w-[220px] truncate text-signal hover:underline"
          >
            {label}
          </button>
          </>
        );
      })}
    </nav>
  );
}
