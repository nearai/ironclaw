export const defaultRoute = "/chat";

export const primaryRoutes = [
  { id: "chat", path: "/chat", labelKey: "nav.chat" },
  { id: "workspace", path: "/workspace", labelKey: "nav.workspace" },
  { id: "projects", path: "/projects", labelKey: "nav.projects" },
  { id: "jobs", path: "/jobs", labelKey: "nav.jobs" },
  { id: "routines", path: "/routines", labelKey: "nav.routines" },
  { id: "missions", path: "/missions", labelKey: "nav.missions" },
  { id: "extensions", path: "/extensions", labelKey: "nav.extensions" },
  { id: "settings", path: "/settings", labelKey: "nav.settings" },
  { id: "admin", path: "/admin", labelKey: "nav.admin" },
];

export const routeSectionDefs = [
  {
    labelKey: "nav.sectionWork",
    ids: ["chat", "workspace", "projects", "jobs", "routines", "missions"],
  },
  {
    labelKey: "nav.sectionSystem",
    ids: ["extensions", "settings", "admin"],
  },
];

export const SETTINGS_SUB_ROUTES = [
  { id: "inference", labelKey: "settings.inference", icon: "spark" },
  { id: "agent", labelKey: "settings.agent", icon: "bolt" },
  { id: "channels", labelKey: "settings.channels", icon: "send" },
  { id: "networking", labelKey: "settings.networking", icon: "pulse" },
  { id: "tools", labelKey: "settings.tools", icon: "tool" },
  { id: "skills", labelKey: "settings.skills", icon: "file" },
  { id: "users", labelKey: "settings.users", icon: "lock" },
  { id: "language", labelKey: "settings.language", icon: "globe" },
];

export const EXTENSIONS_SUB_ROUTES = [
  { id: "installed", labelKey: "extensions.installed", icon: "bolt" },
  { id: "channels", labelKey: "extensions.channels", icon: "send" },
  { id: "mcp", labelKey: "extensions.mcp", icon: "pulse" },
  { id: "registry", labelKey: "extensions.registry", icon: "plus" },
];

export const ADMIN_SUB_ROUTES = [
  { id: "dashboard", labelKey: "admin.tab.dashboard", icon: "pulse" },
  { id: "users", labelKey: "admin.tab.users", icon: "lock" },
  { id: "usage", labelKey: "admin.tab.usage", icon: "spark" },
];

export const EXPANDABLE_SUB_ROUTES = {
  settings: SETTINGS_SUB_ROUTES,
  extensions: EXTENSIONS_SUB_ROUTES,
  admin: ADMIN_SUB_ROUTES,
};

export function routeForId(id) {
  return primaryRoutes.find((route) => route.id === id) || primaryRoutes[0];
}
