/**
 * tokens.ts — machine-readable catalog of the v2 design tokens.
 *
 * The canonical *values* live in src/styles/app.css as `--v2-*`
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
      { var: "--v2-code-bg", note: "Inline code + code block surface" },
    ],
  },
  {
    group: "Cards & borders",
    tokens: [
      { var: "--v2-card-bg", note: "Card/panel surface" },
      { var: "--v2-card-border", note: "Card border (hairline in both themes)" },
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
      { var: "--v2-inverse", note: "Text on inverse fills" },
      { var: "--v2-on-accent", note: "Text on accent/brand fills (white, both themes)" },
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
    group: "Action ramp, glass & scrims",
    tokens: [
      { var: "--v2-btn-primary-bg", note: "Primary button gradient (brand ramp)" },
      { var: "--v2-btn-primary-bg-hover", note: "Primary hover gradient" },
      { var: "--v2-btn-primary-border", note: "Primary button border" },
      { var: "--v2-btn-secondary-border", note: "Outline button border" },
      { var: "--v2-btn-secondary-bg", note: "Outline button frosted fill" },
      { var: "--v2-glass-bg", note: "Glass chrome fill (with --v2-glass-blur)" },
      { var: "--v2-scrim", note: "Modal backdrop dim" },
      { var: "--v2-scrim-soft", note: "Side sheet / task panel scrim" },
    ],
  },
];

/**
 * Status color canon — THE one unambiguous mapping from product
 * status words to semantic tokens (and Badge tones). Every surface
 * that renders a run/job/automation status must draw text, dots, and
 * progress fills from the same pair; never mix (e.g.) info text with
 * a cyan progress bar.
 */
export const STATUS_CANON = [
  { status: "ok / success / completed", tone: "success", text: "--v2-positive-text", fill: "--v2-positive-soft" },
  { status: "running / in progress / active", tone: "info", text: "--v2-info-text", fill: "--v2-info-soft" },
  { status: "warning / degraded / attention", tone: "warning", text: "--v2-warning-text", fill: "--v2-warning-soft" },
  { status: "failure / error / cancelled", tone: "danger", text: "--v2-danger-text", fill: "--v2-danger-soft" },
  { status: "paused / idle / disabled", tone: "muted", text: "--v2-text-muted", fill: "--v2-surface-soft" },
];

/** Control-density scale — shared heights/paddings for interactive
 * controls (Button, Input, Select, …) so mixed rows align. */
export const CONTROL_TOKENS = [
  { var: "--v2-control-h-sm", note: "28px — toolbars, table rows, chips" },
  { var: "--v2-control-h-md", note: "32px — default controls" },
  { var: "--v2-control-h-lg", note: "36px — prominent CTAs, hero forms" },
  { var: "--v2-control-px-sm", note: "Horizontal padding, sm controls" },
  { var: "--v2-control-px-md", note: "Horizontal padding, md controls" },
  { var: "--v2-control-px-lg", note: "Horizontal padding, lg controls" },
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

/** Shadow tokens — restrained elevation scale, all themed (the
 * light-mode scale is much softer than dark's). Borders do the
 * separation work; shadows only lift the surface: cards get a 1px
 * lift, menus/popovers/tooltips share --v2-shadow-menu, and modal
 * dialogs sit one step higher on --v2-shadow-modal. */
export const SHADOW_TOKENS = [
  { var: "--v2-shadow-sm", note: "Subtle lift: rows, small chips (themed)" },
  { var: "--v2-shadow-md", note: "Raised controls (themed)" },
  { var: "--v2-shadow-lg", note: "Floating panels, sheets (themed)" },
  { var: "--v2-card-shadow", note: "Card elevation: minimal 1px lift (themed)" },
  { var: "--v2-shadow-menu", note: "Menus, popovers, tooltips, toasts (themed)" },
  { var: "--v2-shadow-modal", note: "Modal / dialog elevation (themed)" },
  { var: "--v2-shadow-accent-hover", note: "Primary button hover glow" },
];

/** Motion tokens — the nux restrained-motion system (see DESIGN_SYSTEM.md). */
export const MOTION_TOKENS = [
  { var: "--v2-duration-instant", note: "Hover fills, color shifts" },
  { var: "--v2-duration-exit", note: "Menu/overlay exits — quicker than entry" },
  { var: "--v2-duration-fast", note: "Borders, small transforms, press" },
  { var: "--v2-duration-menu", note: "Dropdown/popover entrances" },
  { var: "--v2-duration-base", note: "Panel/sheet/modal entrances" },
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
