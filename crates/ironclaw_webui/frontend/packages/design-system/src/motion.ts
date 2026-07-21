/**
 * Motion constants for the design-system components that animate with
 * `motion/react` (framer-motion).
 *
 * These MIRROR the CSS motion tokens in styles/app.css (--v2-duration-* /
 * --v2-ease-*) — framer-motion needs numbers, CSS transitions need custom
 * properties, so the values live in both places. If you change one, change
 * the other (tokens.test.ts pins the CSS side; keep the comments in sync).
 *
 * Vocabulary (per the restrained-motion policy, DESIGN_SYSTEM.md):
 * - Entrances are quick ease-out; exits are QUICKER than entrances.
 * - Menus/popovers scale in from ~0.96 at the trigger's corner.
 * - Modals fade the scrim and scale/translate the panel from center.
 * - Nothing bounces on pointer or keyboard interactions.
 */
import { useReducedMotion } from "motion/react";

/** Seconds. Mirrors --v2-duration-* (app.css). */
export const MOTION_DURATION = {
  /** --v2-duration-instant (100ms) — hover fills, color shifts. */
  instant: 0.1,
  /** --v2-duration-exit (120ms) — menu/overlay exits, quicker than entry. */
  exit: 0.12,
  /** --v2-duration-fast (150ms) — borders, small transforms, press. */
  fast: 0.15,
  /** --v2-duration-menu (180ms) — dropdown/popover entrances. */
  menu: 0.18,
  /** --v2-duration-base (250ms) — panel/sheet/modal entrances. */
  base: 0.25,
};

/** Mirrors --v2-ease-out-expo — strong ease-out for entrances. */
export const MOTION_EASE_OUT: [number, number, number, number] = [0.16, 1, 0.3, 1];

/** Mirrors --v2-ease-in-out — symmetric moves already on screen. */
export const MOTION_EASE_IN_OUT: [number, number, number, number] = [0.4, 0, 0.2, 1];

/**
 * Reduced-motion hook re-export so design-system components share one
 * import site. Under prefers-reduced-motion, movement is dropped but
 * opacity fades are kept (gentler, not zero — see the Motion policy in
 * app.css and Emil Kowalski's reduced-motion guidance).
 */
export { useReducedMotion };
