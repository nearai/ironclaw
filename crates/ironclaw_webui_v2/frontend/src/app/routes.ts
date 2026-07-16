export const defaultRoute = "/chat";

// `hidden: true` keeps the route registered (direct URL access and
// breadcrumb/title resolution still work) but suppresses it from
// sidebar navigation. Routes whose page-level API libs are entirely
// TODO stubs against missing v2 endpoints are hidden here until the
// matching `/api/webchat/v2/*` contracts land. Remove the flag once
// the page's `lib/*-api.ts` calls real endpoints.
export const primaryRoutes = [
  { id: "chat", path: "/chat", labelKey: "nav.chat" },
  { id: "workspace", path: "/workspace", labelKey: "nav.workspace" },
  // Surfaced in the conversations panel (under Search, above Recent) by
  // SidebarThreads rather than the primary nav list, so `hidden: true` keeps the
  // /projects route registered (direct URL + breadcrumb/title resolution) while
  // suppressing the now-duplicate top-nav entry. Its lib/projects-api.ts calls
  // the real v2 `/api/webchat/v2/projects` endpoints (list/create/read/update/
  // delete + membership ACL); per-project missions/threads remain stubbed.
  { id: "projects", path: "/projects", labelKey: "nav.projects", hidden: true },
  { id: "jobs", path: "/jobs", labelKey: "nav.jobs", hidden: true },
  { id: "routines", path: "/routines", labelKey: "nav.routines", hidden: true },
  { id: "automations", path: "/automations", labelKey: "nav.automations" },
  { id: "missions", path: "/missions", labelKey: "nav.missions", hidden: true },
  { id: "extensions", path: "/extensions", labelKey: "nav.extensions" },
  { id: "logs", path: "/logs", labelKey: "nav.logs", hidden: true },
  { id: "settings", path: "/settings", labelKey: "nav.settings", hidden: false },
  // Un-hidden: its lib/admin-api.ts now calls the real v2
  // `/api/webchat/v2/admin/users*` endpoints (user CRUD + status/role +
  // per-user secret provisioning). Authorization is enforced server-side, so a
  // non-admin caller sees a 403/forbidden state rather than the surface.
  { id: "admin", path: "/admin", labelKey: "nav.admin", hidden: false },
];

export const routeSectionDefs = [
  {
    labelKey: "nav.sectionWork",
    ids: ["chat", "workspace", "projects", "jobs", "routines", "automations", "missions"],
  },
  {
    labelKey: "nav.sectionSystem",
    ids: ["extensions", "logs", "settings", "admin"],
  },
];

export const SETTINGS_SUB_ROUTES = [
  // Inference is un-hidden: its lib/*-api.ts (LLM providers) now calls the real
  // v2 `/api/webchat/v2/llm/*` endpoints, per the unhide rule in the header
  // comment above. The rest stay hidden until their api libs leave stub state.
  { id: "inference", labelKey: "settings.inference", icon: "spark" },
  // Appearance is browser-local UI preference state and does not need a v2 API.
  { id: "appearance", labelKey: "settings.appearance", icon: "sun" },
  // { id: "agent", labelKey: "settings.agent", icon: "bolt" },
  // { id: "channels", labelKey: "settings.channels", icon: "send" },
  // { id: "networking", labelKey: "settings.networking", icon: "pulse" },
  { id: "tools", labelKey: "settings.tools", icon: "tool" },
  { id: "skills", labelKey: "settings.skills", icon: "file" },
  // Trace Commons is un-hidden: its api lib calls the real v2
  // `/api/webchat/v2/traces/credit` endpoint.
  { id: "traces", labelKey: "settings.traceCommons", icon: "layers" },
  // { id: "users", labelKey: "settings.users", icon: "lock" },
  { id: "language", labelKey: "settings.language", icon: "globe" },
];

export const EXTENSIONS_SUB_ROUTES = [
  { id: "registry", labelKey: "extensions.registry", icon: "plus" },
  { id: "channels", labelKey: "extensions.channels", icon: "send" },
  { id: "mcp", labelKey: "extensions.mcp", icon: "pulse" },
];

// Only the Users tab ships in this admin port. The dashboard and usage tabs
// are usage/analytics surfaces, deliberately out of scope here (their
// components remain in the tree but are not routed).
export const ADMIN_SUB_ROUTES = [
  { id: "users", labelKey: "admin.tab.users", icon: "lock" },
];

export const EXPANDABLE_SUB_ROUTES = {
  settings: SETTINGS_SUB_ROUTES,
  extensions: EXTENSIONS_SUB_ROUTES,
  admin: ADMIN_SUB_ROUTES,
};

export function routeForId(id) {
  return primaryRoutes.find((route) => route.id === id) || primaryRoutes[0];
}
