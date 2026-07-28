import { Breadcrumb } from "@ironclaw/ui";
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

  const items = [
    {
      key: "/workspace",
      label: t("workspace.breadcrumbRoot"),
      onSelect: () => onNavigate("/workspace"),
    },
    ...parts.map((part, index) => {
      current = current ? `${current}/${part}` : part;
      const target = current;
      return {
        key: target,
        label: index === 0 ? areaDisplayName(part, t) : part,
        onSelect: () => onNavigate(routeForWorkspacePath(target)),
      };
    }),
  ];

  return (<Breadcrumb label={t("workspace.breadcrumbRoot")} items={items} />);
}
