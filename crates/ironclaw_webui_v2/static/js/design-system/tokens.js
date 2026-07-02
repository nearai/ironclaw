/**
 * tokens.js — machine-readable catalog of the v2 design tokens.
 *
 * The canonical *values* live in static/styles/app.css as `--v2-*`
 * custom properties (light + dark themes) — backfilled from the
 * `achal/nux` gateway exploration and the onboarding prototype (see
 * the provenance header in app.css and DESIGN_SYSTEM.md). This module
 * is the canonical *index* of those tokens: every token the system
 * exposes, grouped by role, with a short usage note. It powers the
 * /playground token pages and gives agents/tooling one import to
 * enumerate the system.
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
  {
    group: "Action ramp & glass",
    tokens: [
      { var: "--v2-btn-primary-bg", note: "Primary button gradient (brand ramp)" },
      { var: "--v2-btn-primary-bg-hover", note: "Primary hover gradient" },
      { var: "--v2-btn-primary-border", note: "Primary button border" },
      { var: "--v2-btn-secondary-border", note: "Outline button border" },
      { var: "--v2-btn-secondary-bg", note: "Outline button frosted fill" },
      { var: "--v2-glass-bg", note: "Glass chrome fill (with --v2-glass-blur)" },
    ],
  },
];

/** Radius scale — nux 6/8/10/16/20/24. */
export const RADIUS_TOKENS = [
  { var: "--v2-radius-xs", note: "Inline chips, skeleton bars" },
  { var: "--v2-radius-sm", note: "Small chips, code spans" },
  { var: "--v2-radius-md", note: "Compact controls, inputs (mobile)" },
  { var: "--v2-radius-lg", note: "Buttons, inputs (desktop), cards" },
  { var: "--v2-radius-xl", note: "Large cards, composer (mobile)" },
  { var: "--v2-radius-2xl", note: "Modals, hero surfaces" },
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

/** Type scale — nux gateway scale. `sample` renders on the typography page. */
export const TYPE_TOKENS = [
  { var: "--v2-font-size-label", note: "Tags, badges, eyebrows (11px)", sample: "TRACE COMMONS" },
  { var: "--v2-font-size-caption", note: "Hints, meta rows, errors (12px)", sample: "Last synced 2 minutes ago" },
  { var: "--v2-font-size-body-sm", note: "Controls + body, mobile (13px)", sample: "Every run is scoped to your project." },
  { var: "--v2-font-size-body", note: "Controls + body, desktop (14px)", sample: "Every run is scoped to your project." },
  { var: "--v2-font-size-body-lg", note: "Descriptions, empty states (16px)", sample: "Connect a channel to start routing messages." },
  { var: "--v2-font-size-title", note: "Modal and panel titles (20px)", sample: "Configure extension" },
  { var: "--v2-font-size-heading", note: "Section headings (24px)", sample: "Recent activity" },
  { var: "--v2-font-size-display-sm", note: "Stat values (28px)", sample: "1,284" },
  { var: "--v2-font-size-display", note: "Page headings, desktop (36px)", sample: "Automations" },
];

/** Shadow tokens — nux layered elevation scale. */
export const SHADOW_TOKENS = [
  { var: "--v2-shadow-sm", note: "Subtle lift: rows, small chips" },
  { var: "--v2-shadow-md", note: "Raised controls, popovers" },
  { var: "--v2-shadow-lg", note: "Floating panels, sheets" },
  { var: "--v2-card-shadow", note: "Card elevation (themed per mode)" },
  { var: "--v2-shadow-modal", note: "Modal / dialog elevation" },
  { var: "--v2-shadow-accent-hover", note: "Primary button hover glow" },
];

/** Motion tokens — the nux restrained-motion system (see DESIGN_SYSTEM.md). */
export const MOTION_TOKENS = [
  { var: "--v2-duration-instant", note: "Hover fills, color shifts" },
  { var: "--v2-duration-fast", note: "Borders, small transforms" },
  { var: "--v2-duration-base", note: "Panel/sheet entrances" },
  { var: "--v2-duration-slow", note: "Large surface transitions" },
  { var: "--v2-ease-standard", note: "Default ease for feedback" },
  { var: "--v2-ease-in-out", note: "Symmetric moves" },
  { var: "--v2-ease-out-expo", note: "Surface entrances" },
  { var: "--v2-ease-spring", note: "Playful pops (chips, badges)" },
  { var: "--v2-ease-spring-gentle", note: "Soft spring settle" },
  { var: "--v2-duration-spin", note: "Loading spinner loop" },
  { var: "--v2-duration-typing", note: "Typing indicator loop" },
  { var: "--v2-duration-breathe", note: "Live badge dot loop" },
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
