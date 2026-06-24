import { NavLink, useLocation } from "react-router";
import { React, html } from "../lib/html.js";
import { primaryRoutes, EXPANDABLE_SUB_ROUTES } from "../app/routes.js";
import { Icon } from "../design-system/icons.js";
import { useT } from "../lib/i18n.js";
import { cn } from "../utils/cn.js";
import { TeeShield } from "./tee-shield.js";

const DOCS_URL = "https://docs.ironclaw.com";

export function PageHeader({ threadsState, onToggleSidebar }) {
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

  return html`
    <header
      className=${cn(
        "flex h-14 shrink-0 items-center gap-3 px-4",
        "border-b border-[var(--v2-panel-border)]",
        "bg-[color-mix(in_srgb,var(--v2-canvas-strong)_88%,transparent)] backdrop-blur-xl"
      )}
    >
      <button
        onClick=${onToggleSidebar}
        className="grid h-8 w-8 shrink-0 place-items-center rounded-[8px] text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] md:hidden"
        aria-label="Toggle sidebar"
      >
        <${Icon} name="list" className="h-4 w-4" />
      </button>

      ${breadcrumb
        ? html`
            <div className="flex min-w-0 items-center gap-2 text-[14px] font-semibold">
              <span className="shrink-0 text-[var(--v2-text-muted)]">
                ${breadcrumb.parent}
              </span>
              <${Icon}
                name="chevron"
                className="h-3.5 w-3.5 shrink-0 -rotate-90 text-[var(--v2-text-muted)]"
              />
              <span className="truncate text-[var(--v2-text-strong)]">
                ${breadcrumb.current}
              </span>
            </div>
          `
        : html`
            <span
              className="truncate text-[14px] font-semibold text-[var(--v2-text-strong)]"
            >
              ${title}
            </span>
          `}

      <div className="ml-auto flex shrink-0 items-center gap-1">
        <${TeeShield} />
        <${NavLink}
          to="/logs"
          className=${({ isActive }) =>
            cn(
              "inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",
              isActive && "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
            )}
          title=${t("nav.logs")}
        >
          ${t("nav.logs")}
        <//>
        <a
          href=${DOCS_URL}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex h-8 items-center rounded-[8px] px-2.5 text-xs font-semibold text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
          title=${t("nav.docs")}
        >
          ${t("nav.docs")}
        </a>
      </div>
    </header>
  `;
}
