/**
 * Shared overlay surface + menu item classes for the extras kit.
 *
 * Internal module (not exported from the extras barrel). Mirrors the popover
 * styling established by the core SelectMenu so every floating surface in the
 * package looks identical: same border/backdrop color-mix, same shadow, same
 * item highlight treatment. Radix-driven items key their hover/focus state off
 * data-[highlighted]; plain listboxes pass explicit active classes instead.
 */

/** Floating panel: menus, popovers, hover cards, comboboxes. */
export const OVERLAY_SURFACE_CLASSES =
  "z-50 overflow-hidden rounded-[10px] " +
  "border border-[color-mix(in_srgb,var(--v2-text-strong)_16%,var(--v2-panel-border))] " +
  "bg-[color-mix(in_srgb,var(--v2-canvas-strong)_92%,var(--v2-surface))] p-1 " +
  "shadow-[0_30px_72px_-18px_rgba(0,0,0,0.86),0_10px_24px_-18px_rgba(0,0,0,0.68)] " +
  "ring-1 ring-[color-mix(in_srgb,var(--v2-text-strong)_8%,transparent)]";

/** Interactive row inside a Radix menu (data-highlighted driven). */
export const MENU_ITEM_CLASSES =
  "relative flex w-full cursor-default select-none items-center gap-2 " +
  "rounded-[7px] px-2.5 py-1.5 text-left text-ui text-[var(--v2-text)] " +
  "outline-none transition-colors " +
  "data-[highlighted]:bg-[var(--v2-surface-muted)] data-[highlighted]:text-[var(--v2-text-strong)] " +
  "data-[disabled]:pointer-events-none data-[disabled]:opacity-50";

/** Non-interactive section label inside a menu. */
export const MENU_LABEL_CLASSES =
  "px-2.5 py-1.5 text-ui-sm font-medium text-[var(--v2-text-faint)]";

/** Thin divider between menu sections. */
export const MENU_SEPARATOR_CLASSES =
  "-mx-1 my-1 h-px bg-[var(--v2-panel-border)]";

/** Keyboard shortcut hint aligned to the right edge of a menu item. */
export const MENU_SHORTCUT_CLASSES =
  "ml-auto pl-4 text-ui-sm tracking-widest text-[var(--v2-text-faint)]";
