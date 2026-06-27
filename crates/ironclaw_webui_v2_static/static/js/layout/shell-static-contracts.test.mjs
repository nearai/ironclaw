import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

function source(relativePath) {
  return readFileSync(new URL(relativePath, import.meta.url), "utf8");
}

function assertIncludes(haystack, needles, label) {
  for (const needle of needles) {
    assert.ok(
      haystack.includes(needle),
      `${label} should include ${JSON.stringify(needle)}`
    );
  }
}

test("GatewayLayout wires onboarding, shell controls, and shared surfaces", () => {
  const layout = source("./gateway-layout.js");

  assertIncludes(
    layout,
    [
      'import { Navigate, Outlet, useLocation, useNavigate } from "react-router";',
      'import { useInterfaceTheme } from "../design-system/theme.js";',
      'import { shouldRouteToOnboarding } from "../lib/onboarding-gate.js";',
      'import { useSidebar } from "../hooks/useSidebar.js";',
      'import { useThreads } from "../pages/chat/hooks/useThreads.js";',
      'import { Sidebar } from "../components/sidebar.js";',
      'import { PageHeader } from "../components/page-header.js";',
      'import { CommandPalette } from "../components/command-palette.js";',
      'import { ToastViewport } from "../components/toast-viewport.js";',
    ],
    "GatewayLayout imports"
  );

  assertIncludes(
    layout,
    [
      "isAdmin &&",
      "shouldRouteToOnboarding({",
      'location.pathname === "/welcome"',
      'location.pathname.startsWith("/settings")',
      '<${Navigate} to="/welcome" replace />',
      '(event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k"',
      "event.preventDefault();",
      "setPaletteOpen((open) => !open);",
      'aria-label=${t("nav.close")}',
      "md:hidden",
      'navigate("/chat", { replace: true });',
      'toast(deleteThreadErrorMessage(error, t), { tone: "error" });',
      "gatewayStatusQuery: statusQuery",
      "threadsState",
    ],
    "GatewayLayout behavior"
  );
});

test("CommandPalette supports core actions, route jumps, thread jumps, and modal UX", () => {
  const palette = source("../components/command-palette.js");

  assertIncludes(
    palette,
    [
      '{ id: "new-chat", label: "New chat"',
      '{ id: "go-chat", label: "Go to Chat"',
      'run: () => navigate("/chat")',
      '{ id: "go-extensions", label: "Go to Extensions"',
      'run: () => navigate("/extensions")',
      '{ id: "go-settings", label: "Go to Settings"',
      'run: () => navigate("/settings")',
      '{ id: "toggle-theme", label: "Toggle theme"',
      "run: () => onToggleTheme?.()",
      "id: `thread-${thread.id}`",
      "run: () => navigate(`/chat/${thread.id}`)",
      "onClose();",
      "command.run();",
      'role="dialog"',
      'aria-modal="true"',
      'aria-label="Command palette"',
      'event.key === "Escape"',
    ],
    "CommandPalette"
  );
});

test("Sidebar navigation filters hidden, chat, and admin-only routes", () => {
  const nav = source("../components/sidebar-nav.js");

  assertIncludes(
    nav,
    [
      'primaryRoutes.filter((r) => r.id !== "chat" && !r.hidden)',
      'navRoutes.filter((route) => isAdmin || route.id !== "admin")',
      '!(route.id === "settings" && ["users", "inference"].includes(subRoute.id))',
      "const defaultPath = `${route.path}/${subRoutes[0].id}`;",
      'data-testid="new-chat"',
      'disabled=${isCreating}',
    ],
    "SidebarNav"
  );
});

test("SidebarThreads preserves explicit pinning, search, deletion, and state badges", () => {
  const threads = source("../components/sidebar-threads.js");

  assertIncludes(
    threads,
    [
      'import { getPinnedIds, subscribePins, togglePin } from "../lib/pin-store.js";',
      "The active thread is no longer auto-pinned",
      "const pinnedList = [];",
      "const recentList = [];",
      "pinnedIds.has(thread.id)",
      'window.confirm("Delete this chat?")',
      "togglePin(thread.id);",
      "THREAD_STATE.NEEDS_ATTENTION",
      "THREAD_STATE.RUNNING",
      "THREAD_STATE.FAILED",
      "totalMatches === 0",
      't("common.noChatsMatch").replace("{query}", query)',
      "rebornProjectsEnabled &&",
      'to="/projects"',
    ],
    "SidebarThreads"
  );
});

test("SidebarFooter and PageHeader expose session, theme, docs, logs, and TEE controls", () => {
  const footer = source("../components/sidebar-footer.js");
  const header = source("../components/page-header.js");

  assertIncludes(
    footer,
    [
      "function profileName(profile)",
      "profile?.display_name || profile?.email || profile?.id ||",
      "useAccountPopover",
      "onClick=${accountPopover.toggle}",
      "onClick=${toggleTheme}",
      'name=${theme === "dark" ? "sun" : "moon"}',
      "onClick=${onSignOut}",
      'name="logout"',
    ],
    "SidebarFooter"
  );

  assertIncludes(
    header,
    [
      'const DOCS_URL = "https://docs.ironclaw.com";',
      'aria-label="Toggle sidebar"',
      'aria-controls="gateway-sidebar"',
      "EXPANDABLE_SUB_ROUTES[route.id]",
      "threadsState.activeThreadId",
      "<${TeeShield} />",
      'to="/logs"',
      "href=${DOCS_URL}",
      'rel="noopener noreferrer"',
    ],
    "PageHeader"
  );
});

test("ToastViewport and TeeShield retain async feedback and report controls", () => {
  const toastViewport = source("../components/toast-viewport.js");
  const teeShield = source("../components/tee-shield.js");

  assertIncludes(
    toastViewport,
    [
      'import { subscribeToasts } from "../lib/toast.js";',
      "setItems((prev) => [...prev, item]);",
      "setTimeout(",
      'role="status"',
      "TONE[item.tone] || TONE.info",
      'name=${ICON[item.tone] || "bolt"}',
    ],
    "ToastViewport"
  );

  assertIncludes(
    teeShield,
    [
      "useTeeAttestation()",
      "if (nextOpen) tee.loadReport();",
      "tee.copyReport().catch(() => {});",
      "if (!tee.available) return null;",
      "const rows = buildRows({ teeInfo: tee.teeInfo, report: tee.report, t });",
      "tee.reportLoading",
      "tee.reportError",
      "tee.copied ? t(\"tee.copied\") : t(\"tee.copyReport\")",
      "function summarizeValue(value)",
      "text.length > 72",
    ],
    "TeeShield"
  );
});
