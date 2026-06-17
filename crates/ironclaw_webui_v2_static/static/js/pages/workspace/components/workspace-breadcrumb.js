import { html } from "../../../lib/html.js";
import {
  areaDisplayName,
  pathSegments,
  routeForWorkspacePath,
  WORKSPACE_ROOT_LABEL,
} from "../lib/workspace-presenters.js";

// Path breadcrumb shared by the file viewer and the directory listing. The root
// is shown as "workspace" (both areas live under it); the first segment is a
// storage area, rendered by its display name ("home"/"memory") while still
// navigating by its real id. Every crumb uses the same URL-as-state path the
// tree uses, so breadcrumb clicks, tree clicks, and direct links stay in sync.
export function WorkspaceBreadcrumb({ path, onNavigate }) {
  const parts = pathSegments(path);
  let current = "";

  return html`
    <div className="flex min-w-0 flex-wrap items-center gap-2 font-mono text-sm">
      <button
        type="button"
        onClick=${() => onNavigate("/workspace")}
        className="text-signal hover:underline"
      >
        ${WORKSPACE_ROOT_LABEL}
      </button>
      ${parts.map((part, index) => {
        current = current ? `${current}/${part}` : part;
        const target = current;
        const label = index === 0 ? areaDisplayName(part) : part;
        return html`
          <span key=${target} className="text-iron-400">/</span>
          <button
            key=${`${target}-button`}
            type="button"
            onClick=${() => onNavigate(routeForWorkspacePath(target))}
            className="max-w-[220px] truncate text-signal hover:underline"
          >
            ${label}
          </button>
        `;
      })}
    </div>
  `;
}
