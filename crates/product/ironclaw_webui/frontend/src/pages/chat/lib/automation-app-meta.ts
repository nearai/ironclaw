/**
 * Presentation metadata per third-party app — the icon, i18n label key, and a
 * brand accent used to tint the app chip so a completed task is recognizable at
 * a glance. Kept separate from the data model so the wire contract carries only
 * the `AutomationApp` enum, not UI styling.
 */
import type { AutomationApp } from "./automation-tasks";

export interface AutomationAppMeta {
  icon: string;
  labelKey: string;
  /** Brand accent for the icon chip (foreground); background stays neutral. */
  accent: string;
}

const APP_META: Record<AutomationApp, AutomationAppMeta> = {
  gmail: { icon: "mail", labelKey: "automation.app.gmail", accent: "#ea4335" },
  google_calendar: {
    icon: "calendar",
    labelKey: "automation.app.calendar",
    accent: "#4285f4",
  },
  google_docs: {
    icon: "file",
    labelKey: "automation.app.docs",
    accent: "#3b82f6",
  },
  slack: { icon: "chat", labelKey: "automation.app.slack", accent: "#e01e5a" },
  notion: {
    icon: "bookOpen",
    labelKey: "automation.app.notion",
    accent: "#9aa4b2",
  },
};

const FALLBACK: AutomationAppMeta = {
  icon: "spark",
  labelKey: "automation.app.unknown",
  accent: "#9aa4b2",
};

export function appMeta(app: AutomationApp): AutomationAppMeta {
  return APP_META[app] ?? FALLBACK;
}

/** Inline style for the app icon chip: brand-tinted foreground, soft fill. */
export function appChipStyle(app: AutomationApp): {
  color: string;
  background: string;
  borderColor: string;
} {
  const { accent } = appMeta(app);
  return {
    color: accent,
    background: `color-mix(in srgb, ${accent} 16%, transparent)`,
    borderColor: `color-mix(in srgb, ${accent} 38%, transparent)`,
  };
}
