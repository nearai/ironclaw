/**
 * tokens.js — machine-readable catalog of the v2 design tokens.
 *
 * The canonical *values* live in static/styles/app.css as `--v2-*`
 * custom properties (light + dark themes). This module is the
 * canonical *index* of those tokens: every token the system exposes,
 * grouped by role, with a short usage note. It powers the
 * /playground token pages and gives agents/tooling one import to
 * enumerate the system (see DESIGN_SYSTEM.md).
 *
 * Reading a value at runtime:
 *   getComputedStyle(document.documentElement).getPropertyValue("--v2-accent")
 * (theme-aware — resolves against the active [data-theme]).
 */

/** Color tokens, grouped by semantic role. `var` is the CSS custom property. */
export const COLOR_TOKENS = [
  {
    group: "Canvas & surfaces",
    tokens: [
      { var: "--v2-canvas", note: "App background" },
      { var: "--v2-canvas-strong", note: "Deepest background (body, chrome)" },
      { var: "--v2-surface", note: "Raised surface (panels, sidebar)" },
      { var: "--v2-surface-soft", note: "Hover fills, subtle insets" },
      { var: "--v2-surface-muted", note: "Strong insets, code chips, skeletons" },
      { var: "--v2-rail", note: "Rails and input wells (dark)" },
      { var: "--v2-input-bg", note: "Form control background" },
    ],
  },
  {
    group: "Cards & borders",
    tokens: [
      { var: "--v2-card-bg", note: "Card/panel surface" },
      { var: "--v2-card-border", note: "Card border (transparent in dark)" },
      { var: "--v2-panel-border", note: "Default border: dividers, form borders" },
    ],
  },
  {
    group: "Text",
    tokens: [
      { var: "--v2-text", note: "Default body text" },
      { var: "--v2-text-strong", note: "Headings, values, emphasized text" },
      { var: "--v2-text-muted", note: "Secondary text, descriptions" },
      { var: "--v2-text-faint", note: "Placeholders, eyebrow labels" },
      { var: "--v2-inverse", note: "Text on accent/inverse fills" },
      { var: "--v2-ink", note: "Brand ink (logo, marks)" },
    ],
  },
  {
    group: "Accent",
    tokens: [
      { var: "--v2-accent", note: "Primary action color (signal blue)" },
      { var: "--v2-accent-strong", note: "Pressed/gradient end of accent" },
      { var: "--v2-accent-soft", note: "Accent tint fill (selection, hover)" },
      { var: "--v2-accent-text", note: "Accent-colored text and links" },
    ],
  },
  {
    group: "Semantic status",
    tokens: [
      { var: "--v2-positive-soft", note: "Success tint fill" },
      { var: "--v2-positive-text", note: "Success text/icon" },
      { var: "--v2-warning-soft", note: "Warning tint fill" },
      { var: "--v2-warning-text", note: "Warning text/icon" },
      { var: "--v2-danger-soft", note: "Danger tint fill" },
      { var: "--v2-danger-text", note: "Danger text/icon" },
      { var: "--v2-info-soft", note: "Info tint fill" },
      { var: "--v2-info-text", note: "Info text/icon" },
    ],
  },
];

/** Radius scale. */
export const RADIUS_TOKENS = [
  { var: "--v2-radius-xs", note: "Inline chips, skeleton bars" },
  { var: "--v2-radius-sm", note: "Compact (sm) buttons and inputs" },
  { var: "--v2-radius-md", note: "Default controls (mobile)" },
  { var: "--v2-radius-lg", note: "Default controls (desktop)" },
  { var: "--v2-radius-xl", note: "Cards, composer (mobile)" },
  { var: "--v2-radius-2xl", note: "Large cards, modals" },
  { var: "--v2-radius-full", note: "Pills, round icon chips" },
];

/** Spacing scale — 4px base grid. */
export const SPACE_TOKENS = [
  { var: "--v2-space-1", note: "Hairline gaps inside chips" },
  { var: "--v2-space-2", note: "Icon-to-label gaps" },
  { var: "--v2-space-3", note: "Control gaps, button rows" },
  { var: "--v2-space-4", note: "Form-field stacks" },
  { var: "--v2-space-5", note: "Card padding (mobile)" },
  { var: "--v2-space-6", note: "Section gaps" },
  { var: "--v2-space-7", note: "Card padding (desktop)" },
  { var: "--v2-space-8", note: "Page-level gutters" },
  { var: "--v2-space-10", note: "Hero / empty-state breathing room" },
];

/** Type scale. `sample` is rendered on the playground typography page. */
export const TYPE_TOKENS = [
  { var: "--v2-font-size-label", note: "Mono-caps eyebrows, badges", sample: "TRACE COMMONS" },
  { var: "--v2-font-size-caption", note: "Hints, meta rows, errors", sample: "Last synced 2 minutes ago" },
  { var: "--v2-font-size-body-sm", note: "Controls + body (mobile)", sample: "Every run is scoped to your project." },
  { var: "--v2-font-size-body", note: "Controls + body (desktop)", sample: "Every run is scoped to your project." },
  { var: "--v2-font-size-body-lg", note: "Descriptions, empty states", sample: "Connect a channel to start routing messages." },
  { var: "--v2-font-size-title", note: "Modal and panel titles", sample: "Configure extension" },
  { var: "--v2-font-size-heading", note: "Section sub-labels", sample: "Recent activity" },
  { var: "--v2-font-size-display-sm", note: "Stat values", sample: "1,284" },
  { var: "--v2-font-size-display", note: "Page headings (desktop)", sample: "Automations" },
];

/** Shadow tokens. */
export const SHADOW_TOKENS = [
  { var: "--v2-card-shadow", note: "Card elevation (themed: none in light)" },
  { var: "--v2-shadow-modal", note: "Modal / dialog elevation" },
  { var: "--v2-shadow-accent-hover", note: "Primary button hover glow" },
];

/** Motion tokens — see the static-motion policy in DESIGN_SYSTEM.md. */
export const MOTION_TOKENS = [
  { var: "--v2-duration-spin", note: "Loading spinner (sanctioned exception)" },
  { var: "--v2-duration-typing", note: "Typing indicator (sanctioned exception)" },
  { var: "--v2-duration-breathe", note: "Live badge dot (sanctioned exception)" },
  { var: "--v2-ease-standard", note: "Easing for sanctioned motion" },
];

/** Z-index ladder — pick a layer, never a raw number. */
export const Z_TOKENS = [
  { var: "--v2-z-raised", note: "In-page floats (scroll-to-bottom)" },
  { var: "--v2-z-sticky", note: "Pinned headers/banners inside a pane" },
  { var: "--v2-z-overlay", note: "Scrims, off-canvas drawers" },
  { var: "--v2-z-modal", note: "Dialogs, command palette" },
  { var: "--v2-z-toast", note: "Toast viewport (above modals)" },
];

/** Resolve a token's current computed value (theme-aware). */
export function readToken(name) {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}
