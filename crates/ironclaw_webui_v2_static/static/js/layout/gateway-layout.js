import { Navigate, Outlet, useLocation, useNavigate } from "react-router";
import { useInterfaceTheme } from "../design-system/theme.js";
import { useGatewayStatus } from "../hooks/useGatewayStatus.js";
import { useNotifications } from "../hooks/useNotifications.js";
import { useLlmProviders } from "../pages/settings/hooks/useLlmProviders.js";
import { shouldRouteToOnboarding } from "../lib/onboarding-gate.js";
import { useSidebar } from "../hooks/useSidebar.js";
import { html } from "../lib/html.js";
import { useT } from "../lib/i18n.js";
import { toast } from "../lib/toast.js";
import { deleteThreadErrorMessage } from "../lib/thread-errors.js";
import { useThreads } from "../pages/chat/hooks/useThreads.js";
import { Sidebar } from "../components/sidebar.js";
import { PageHeader } from "../components/page-header.js";
import { CommandPalette } from "../components/command-palette.js";
import { ToastViewport } from "../components/toast-viewport.js";
import { React } from "../lib/html.js";
import { cn } from "../utils/cn.js";

export function GatewayLayout({
  token,
  profile,
  isChecking = false,
  isAdmin,
  rebornProjectsEnabled = false,
  globalAutoApproveEnabled = false,
  onSignOut,
}) {
  const t = useT();
  const { theme, toggleTheme } = useInterfaceTheme();
  const statusQuery = useGatewayStatus(token);
  const threadsState = useThreads();
  const notificationsState = useNotifications({
    profile,
    enabled: Boolean(token),
    activeThreadId: threadsState.activeThreadId,
    threads: threadsState.threads,
  });
  const sidebar = useSidebar({
    onNewChat: () => threadsState.setActiveThreadId(null),
  });
  const status = statusQuery.data;

  // First-run gate: with no LLM provider configured yet, route to the welcome
  // screen so the user picks one before hitting a dead chat. Settings stays
  // reachable so they can configure there too; /welcome itself is exempt to
  // avoid a redirect loop. Defaults are not treated as "configured" — the gate
  // keys off the honest `hasActiveProvider` (a persisted selection).
  const location = useLocation();
  const navigate = useNavigate();
  const llmProviders = useLlmProviders({
    settings: {},
    gatewayStatus: status,
    enabled: isAdmin,
  });
  // Onboarding is admin-only; non-admins never see the first-run gate.
  // Even for an admin, skip onboarding when the providers query errored —
  // under multi-user / SSO auth the operator LLM-config route is gated
  // (404), the provider is configured operator-side at boot, and `/welcome`
  // can't reach the gated config UI, so a failed query must not trap an
  // admin SSO user on `/welcome`.
  const needsOnboarding =
    isAdmin &&
    shouldRouteToOnboarding({
      isLoading: llmProviders.isLoading,
      hasActiveProvider: llmProviders.hasActiveProvider,
      isError: llmProviders.isError,
    });
  const onboardingExempt =
    location.pathname === "/welcome" || location.pathname.startsWith("/settings");

  const [paletteOpen, setPaletteOpen] = React.useState(false);
  React.useEffect(() => {
    const onKeyDown = (event) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPaletteOpen((open) => !open);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const handleDeleteThread = React.useCallback(
    async (threadId) => {
      const wasActive = threadsState.activeThreadId === threadId;
      try {
        await threadsState.deleteThread(threadId);
        if (wasActive) {
          navigate("/chat", { replace: true });
        }
      } catch (error) {
        console.error("Failed to delete thread:", error);
        toast(deleteThreadErrorMessage(error, t), { tone: "error" });
      }
    },
    [navigate, threadsState, t]
  );
  if (needsOnboarding && !onboardingExempt) {
    return html`<${Navigate} to="/welcome" replace />`;
  }

  return html`
    <div className="flex h-[100dvh] overflow-hidden bg-[var(--v2-canvas)]">
      ${sidebar.mobileOpen &&
      html`<button
        type="button"
        aria-label=${t("nav.close")}
        onClick=${sidebar.close}
        className="fixed inset-0 z-40 bg-black/40 md:hidden"
      />`}

      <div
        className=${cn(
          "fixed inset-y-0 left-0 z-50 md:relative md:z-auto",
          sidebar.mobileOpen ? "flex" : "hidden",
          sidebar.desktopOpen ? "md:flex" : "md:hidden"
        )}
      >
        <${Sidebar}
          id="gateway-sidebar"
          threadsState=${threadsState}
          theme=${theme}
          toggleTheme=${toggleTheme}
          profile=${profile}
          isAdmin=${isAdmin}
          rebornProjectsEnabled=${rebornProjectsEnabled}
          onSignOut=${onSignOut}
          onClose=${sidebar.close}
          onNewChat=${sidebar.newChat}
          onSelectThread=${sidebar.selectThread}
          onDeleteThread=${handleDeleteThread}
        />
      </div>

      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <${PageHeader}
          threadsState=${threadsState}
          notificationsState=${notificationsState}
          onToggleSidebar=${sidebar.toggle}
          sidebarOpen=${sidebar.currentOpen}
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
              isChecking,
              isAdmin,
              globalAutoApproveEnabled,
              threadsState,
            }}
          />
        </main>
      </div>
      <${CommandPalette}
        open=${paletteOpen}
        onClose=${() => setPaletteOpen(false)}
        threadsState=${threadsState}
        onNewChat=${sidebar.newChat}
        onToggleTheme=${toggleTheme}
      />
      <${ToastViewport} />
    </div>
  `;
}
