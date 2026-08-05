import {
  isDesktopSidebarViewport,
  type ViewportQuerySource,
} from "../../../lib/sidebar-state";

// Programmatic focus on a phone pops the virtual keyboard over the transcript
// the user just navigated to. Same 768px breakpoint as the sidebar on purpose:
// "desktop" is one layout decision, not two that can drift apart.
export function shouldAutoFocusComposer(win: ViewportQuerySource): boolean {
  return isDesktopSidebarViewport(win);
}

// Focusing the composer runs a frame after navigation, by which time the
// browser has already focused whatever the user clicked to get here. Chrome and
// Firefox focus a <button> on click, so the "+ New" button and the sidebar
// thread rows are *expected* to hold focus when we run — stealing from them is
// the entire point. Deny only the two cases where the user is deliberately
// somewhere else: a modal's focus trap, and text entry in another field.
export function canStealFocus(
  activeElement: Element | null,
  composerNode: HTMLTextAreaElement | null,
): boolean {
  if (activeElement == null) return true;
  if (activeElement === composerNode) return true;
  if (composerNode?.contains(activeElement) === true) return true;
  if (activeElement.closest?.("[role='dialog'], [aria-modal='true']")) {
    return false;
  }
  const tag = activeElement.tagName?.toLowerCase();
  if (tag === "input" || tag === "textarea" || tag === "select") return false;
  if ((activeElement as HTMLElement).isContentEditable === true) return false;
  return true;
}
