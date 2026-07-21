/**
 * Storybook chrome theme derived from the IronClaw design tokens
 * (packages/design-system/src/tokens.css, light theme). The manager UI
 * can't consume CSS custom properties directly, so the canonical hex
 * values are mirrored here — if tokens.css changes, update this file.
 */
import { create } from "storybook/theming";

const FONT_SANS =
  '"Geist", "Inter", "SF Pro Display", "Helvetica Neue", helvetica, arial, sans-serif';
const FONT_MONO =
  '"Geist Mono", "SF Mono", "JetBrains Mono", ui-monospace, monospace';

export const ironclawTheme = create({
  base: "light",

  brandTitle: "IronClaw Design System",
  brandUrl: "https://ironclaw-design-system-demo.vercel.app/playground",
  brandTarget: "_blank",

  fontBase: FONT_SANS,
  fontCode: FONT_MONO,

  // Accent — IronClaw brand blue (--v2-accent / --v2-accent-strong)
  colorPrimary: "#2882c8",
  colorSecondary: "#2882c8",

  // App chrome — sidebar sits on --v2-canvas, content on --v2-surface
  appBg: "#f5f5f7",
  appContentBg: "#ffffff",
  appPreviewBg: "#ffffff",
  appBorderColor: "rgba(0, 0, 0, 0.08)", // --v2-panel-border
  appBorderRadius: 10, // --v2-radius-md

  // Text — --v2-text / --v2-text-muted / --v2-inverse
  textColor: "#1a1a2e",
  textInverseColor: "#ffffff",
  textMutedColor: "#555555",

  // Toolbar
  barBg: "#ffffff",
  barTextColor: "#555555",
  barHoverColor: "#1f6ca8",
  barSelectedColor: "#2882c8",

  // Form controls — --v2-input-bg / --v2-panel-border / --v2-radius-sm
  inputBg: "#ffffff",
  inputBorder: "rgba(0, 0, 0, 0.08)",
  inputTextColor: "#1a1a2e",
  inputBorderRadius: 8,

  buttonBg: "#f5f5f7",
  buttonBorder: "rgba(0, 0, 0, 0.08)",
  booleanBg: "#ebebed",
  booleanSelectedBg: "#ffffff",
});
