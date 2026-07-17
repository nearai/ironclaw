export const DESKTOP_SIDEBAR_STORAGE_KEY = "ironclaw:v2-sidebar-open";

function browserWindow() {
  return typeof window === "undefined" ? null : window;
}

function browserStorage() {
  try {
    return browserWindow()?.localStorage || null;
  } catch (_) {
    return null;
  }
}

export function readDesktopSidebarOpen(storage = browserStorage()) {
  try {
    return storage?.getItem(DESKTOP_SIDEBAR_STORAGE_KEY) !== "false";
  } catch (_) {
    return true;
  }
}

export function writeDesktopSidebarOpen(open, storage = browserStorage()) {
  try {
    storage?.setItem(DESKTOP_SIDEBAR_STORAGE_KEY, open ? "true" : "false");
  } catch (_) {
    // Best-effort: storage failures should never block navigation.
  }
}

export function isDesktopSidebarViewport(win = browserWindow()) {
  try {
    return win?.matchMedia?.("(min-width: 768px)").matches === true;
  } catch (_) {
    return false;
  }
}

export function toggleSidebarState(state, isDesktop) {
  return isDesktop
    ? { ...state, desktopOpen: !state.desktopOpen }
    : { ...state, mobileOpen: !state.mobileOpen };
}

export function currentSidebarOpen(state, isDesktop) {
  return isDesktop ? state.desktopOpen : state.mobileOpen;
}
