import { NavLink, useLocation } from "react-router";
import React from "react";
import { primaryRoutes, EXPANDABLE_SUB_ROUTES } from "../app/routes";
import { Icon } from "../design-system/icons";
import { useT } from "../lib/i18n";
import { cn } from "../utils/cn";
import { TeeShield } from "./tee-shield";
import { NotificationCenter } from "./notification-center";

const DOCS_URL = "https://docs.ironclaw.com";

export function PageHeader({
  threadsState,
  notificationsState,
  status = null,
  onToggleSidebar,
  sidebarOpen = true,
}) {
  const t = useT();
  const location = useLocation();

  const breadcrumb = React.useMemo(() => {
    for (const route of primaryRoutes) {
      const subRoutes = EXPANDABLE_SUB_ROUTES[route.id];
      if (!subRoutes) continue;
      const prefix = route.path + "/";
      if (location.pathname.startsWith(prefix)) {
        const subId = location.pathname.slice(prefix.length).split("/")[0];
        const sub = subRoutes.find((s) => s.id === subId);
        if (sub) {
          return {
            parent: t(route.labelKey),
            current: t(sub.labelKey),
          };
        }
      }
    }
    return null;
  }, [location.pathname, t]);

  const title = React.useMemo(() => {
    if (breadcrumb) return null;
    if (location.pathname.startsWith("/chat")) {
      if (threadsState.activeThreadId) {
        const thread = threadsState.threads.find(
          (th) => th.id === threadsState.activeThreadId
        );
        return thread?.title || t("nav.chat");
      }
      return t("nav.chat");
    }
    const route = primaryRoutes.find((r) =>
      location.pathname.startsWith(r.path)
    );
    return route ? t(route.labelKey) : "";
  }, [location.pathname, threadsState.activeThreadId, threadsState.threads, t, breadcrumb]);

  const toggleSidebarLabel = t("sidebar.toggle");

  return (
    <header
      className={cn(
        "flex h-14 shrink-0 items-center gap-3 px-4",
        "border-b border-[var(--v2-panel-border)]",
        "bg-[color-mix(in_srgb,var(--v2-canvas-strong)_88%,transparent)] backdrop-blur-xl"
      )}
    >
      <button
        type="button"
        onClick={onToggleSidebar}
        className="grid h-8 w-8 shrink-0 place-items-center rounded-[8px] text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)]"
        aria-label={toggleSidebarLabel}
        aria-controls="gateway-sidebar"
        aria-expanded={sidebarOpen ? "true" : "false"}
        title={toggleSidebarLabel}
      >
        <Icon name="list" className="h-4 w-4" />
      </button>

      {breadcrumb
        ? (
            <div className="flex min-w-0 items-center gap-2 text-[14px] font-semibold">
              <span className="shrink-0 text-[var(--v2-text-muted)]">
                {breadcrumb.parent}
              </span>
              <Icon
                name="chevron"
                className="h-3.5 w-3.5 shrink-0 -rotate-90 text-[var(--v2-text-muted)]"
              />
              <span className="truncate text-[var(--v2-text-strong)]">
                {breadcrumb.current}
              </span>
            </div>
          )
        : (
            <span
              className="truncate text-[14px] font-semibold text-[var(--v2-text-strong)]"
            >
              {title}
            </span>
          )}

      <div className="ml-auto flex shrink-0 items-center gap-1">
        {status}
        <NotificationCenter state={notificationsState} />
        <TeeShield />
        <NavLink
          to="/logs"
          data-testid="header-logs-link"
          className={({ isActive }) =>
            cn(
              "grid h-8 w-8 place-items-center rounded-[8px] text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",
              isActive && "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
            )}
          title={t("nav.logs")}
          aria-label={t("nav.logs")}
        >
          <Icon name="terminal" className="h-4 w-4" />
        </NavLink>
        <a
          href={DOCS_URL}
          target="_blank"
          rel="noopener noreferrer"
          data-testid="header-docs-link"
          className="grid h-8 w-8 place-items-center rounded-[8px] text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          title={t("nav.docs")}
          aria-label={t("nav.docs")}
        >
          <Icon name="bookOpen" className="h-4 w-4" />
        </a>
      </div>
    </header>
  );
}
