import { Outlet } from "react-router";
import { useInterfaceTheme } from "../design-system/theme.js";
import { useGatewayStatus } from "../hooks/useGatewayStatus.js";
import { useSidebar } from "../hooks/useSidebar.js";
import { html } from "../lib/html.js";
import { useT } from "../lib/i18n.js";
import { useThreads } from "../pages/chat/hooks/useThreads.js";
import { Sidebar } from "../components/sidebar.js";
import { PageHeader } from "../components/page-header.js";
import { cn } from "../utils/cn.js";

export function GatewayLayout({ token, profile, isAdmin, onSignOut }) {
  const t = useT();
  const { theme, toggleTheme } = useInterfaceTheme();
  const statusQuery = useGatewayStatus(token);
  const threadsState = useThreads();
  const sidebar = useSidebar({
    onNewChat: () => threadsState.setActiveThreadId(null),
  });
  const status = statusQuery.data;
  // v2 has no DELETE thread endpoint, so the sidebar renders no
  // delete affordance (SidebarThreads conditionally renders the
  // trash button on `onDelete`).

  return html`
    <div className="flex h-[100dvh] overflow-hidden bg-[var(--v2-canvas)]">
      ${sidebar.open &&
      html`<button
        type="button"
        aria-label=${t("nav.close")}
        onClick=${sidebar.close}
        className="fixed inset-0 z-40 bg-black/40 md:hidden"
      />`}

      <div
        className=${cn(
          "fixed inset-y-0 left-0 z-50 md:relative md:z-auto",
          sidebar.open ? "flex" : "hidden md:flex"
        )}
      >
        <${Sidebar}
          threadsState=${threadsState}
          theme=${theme}
          toggleTheme=${toggleTheme}
          profile=${profile}
          isAdmin=${isAdmin}
          onSignOut=${onSignOut}
          onClose=${sidebar.close}
          onNewChat=${sidebar.newChat}
          onSelectThread=${sidebar.selectThread}
        />
      </div>

      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <${PageHeader}
          threadsState=${threadsState}
          onToggleSidebar=${sidebar.toggle}
        />
        <main className="min-h-0 min-w-0 flex-1 overflow-hidden">
          ${statusQuery.error &&
          html`
            <div
              className=${cn(
                "m-4 rounded-[14px] border px-4 py-3 text-sm",
                "border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))]",
                "bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]"
              )}
            >
              ${statusQuery.error.message || t("error.gatewayConnection")}
            </div>
          `}
          <${Outlet}
            context=${{
              gatewayStatus: status,
              gatewayStatusQuery: statusQuery,
              currentUser: profile,
              isAdmin,
              threadsState,
            }}
          />
        </main>
      </div>
    </div>
  `;
}
