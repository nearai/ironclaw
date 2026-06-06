import { NavLink, useLocation } from "react-router";
import { primaryRoutes, EXPANDABLE_SUB_ROUTES } from "../app/routes.js";
import { Icon } from "../design-system/icons.js";
import { React, html } from "../lib/html.js";
import { useT } from "../lib/i18n.js";
import { cn } from "../utils/cn.js";

const ROUTE_ICONS = {
  chat: "chat",
  workspace: "layers",
  projects: "folder",
  jobs: "pulse",
  routines: "clock",
  automations: "calendar",
  missions: "flag",
  extensions: "plug",
  settings: "settings",
  admin: "shield",
};

const navRoutes = primaryRoutes.filter((r) => r.id !== "chat" && !r.hidden);

function NavItem({ route, label, onNavigate }) {
  return html`
    <${NavLink}
      to=${route.path}
      onClick=${onNavigate}
      className=${({ isActive }) =>
        cn(
          "flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",
          isActive
            ? "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
            : "text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
        )}
    >
      <${Icon} name=${ROUTE_ICONS[route.id] || "bolt"} className="h-4 w-4 shrink-0" />
      <span className="min-w-0 truncate">${label}</span>
    <//>
  `;
}

function ExpandableNavItem({ route, label, subRoutes, onNavigate }) {
  const t = useT();
  const location = useLocation();
  const isExpanded =
    location.pathname === route.path ||
    location.pathname.startsWith(route.path + "/");
  const defaultPath = `${route.path}/${subRoutes[0].id}`;

  return html`
    <div className="flex flex-col">
      <${NavLink}
        to=${defaultPath}
        onClick=${onNavigate}
        className=${() =>
          cn(
            "flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13px] font-medium",
            isExpanded
              ? "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
              : "text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          )}
      >
        <${Icon}
          name=${ROUTE_ICONS[route.id] || "bolt"}
          className="h-4 w-4 shrink-0"
        />
        <span className="min-w-0 flex-1 truncate">${label}</span>
        <${Icon}
          name="chevron"
          className=${cn(
            "h-3.5 w-3.5 shrink-0 transition-transform duration-150",
            isExpanded && "rotate-180"
          )}
        />
      <//>

      ${isExpanded &&
      html`
        <div className="mt-0.5 flex flex-col gap-0.5 pl-3">
          ${subRoutes.map(
            (sub) => html`
              <${NavLink}
                key=${sub.id}
                to=${route.path + "/" + sub.id}
                onClick=${onNavigate}
                className=${({ isActive }) =>
                  cn(
                    "flex items-center gap-2.5 rounded-[8px] py-1.5 pl-7 pr-3 text-[12px] font-medium",
                    isActive
                      ? "text-[var(--v2-accent-text)]"
                      : "text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
                  )}
              >
                <${Icon} name=${sub.icon} className="h-3 w-3 shrink-0" />
                <span className="min-w-0 truncate">${t(sub.labelKey)}</span>
              <//>
            `
          )}
        </div>
      `}
    </div>
  `;
}

export function SidebarNav({ onNewChat, isCreating, isAdmin = false, onNavigate }) {
  const t = useT();
  const visibleRoutes = React.useMemo(
    () => navRoutes.filter((route) => isAdmin || route.id !== "admin"),
    [isAdmin]
  );

  return html`
    <div className="flex flex-col px-3 py-2">
      <button
        onClick=${onNewChat}
        disabled=${isCreating}
        className=${cn(
          "flex items-center gap-2.5 rounded-[10px] px-3 py-2",
          "border border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",
          "bg-[var(--v2-accent-soft)] text-[13px] font-medium text-[var(--v2-accent-text)]",
          "hover:bg-[color-mix(in_srgb,var(--v2-accent)_18%,transparent)] disabled:opacity-50"
        )}
      >
        <${Icon} name="plus" className="h-4 w-4 shrink-0" />
        <span>${isCreating ? t("chat.creating") : t("chat.newThread")}</span>
      </button>

      <nav className="mt-2 flex flex-col gap-1">
        ${visibleRoutes.map((route) => {
          const subRoutes = (EXPANDABLE_SUB_ROUTES[route.id] || []).filter(
            (subRoute) =>
              isAdmin ||
              !(route.id === "settings" && ["users", "inference"].includes(subRoute.id))
          );
          if (subRoutes.length > 0) {
            return html`
              <${ExpandableNavItem}
                key=${route.id}
                route=${route}
                label=${t(route.labelKey)}
                subRoutes=${subRoutes}
                onNavigate=${onNavigate}
              />
            `;
          }
          return html`
            <${NavItem}
              key=${route.id}
              route=${route}
              label=${t(route.labelKey)}
              onNavigate=${onNavigate}
            />
          `;
        })}
      </nav>
    </div>
  `;
}
