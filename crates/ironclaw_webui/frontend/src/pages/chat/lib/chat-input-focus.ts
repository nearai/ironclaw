import { isDesktopSidebarViewport } from "../../../lib/sidebar-state";

export function shouldAutoFocusComposer(win) {
  return isDesktopSidebarViewport(win);
}

export function canStealFocus(activeElement, composerNode) {
  return (
    activeElement == null ||
    activeElement.tagName?.toLowerCase() === "body" ||
    activeElement === composerNode ||
    composerNode?.contains?.(activeElement) === true
  );
}
