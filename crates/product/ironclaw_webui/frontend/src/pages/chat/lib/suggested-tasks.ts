/**
 * Suggested tasks — the lean model behind the OOBE first-run suggestion cards
 * (PROPOSAL §2A). A card is a status affordance for one proposed piece of work;
 * its `state` drives which single action row renders:
 *
 *   unconnected → Connect <tool>          (authorize the tool first)
 *   suggested   → Approve                 (kick off a foreground turn)
 *   running     → live "working" line     (activity shows in the thread)
 *   completed   → Completed + "+ Automation"
 *   failed      → "Couldn't complete" + Try again
 *
 * Slice 1 is presentational only: no data fetching, no durable events yet — the
 * surface renders a static demo list gated behind the `oobe_suggestions` flag.
 * The wire contract carries only the `AutomationApp` enum; icon/label/brand
 * styling lives here so the durable model never learns about UI.
 */

/** Third-party app a suggested task acts in. Drives the icon and brand tint. */
export type AutomationApp =
  | "gmail"
  | "google_calendar"
  | "google_docs"
  | "slack"
  | "notion";

/** Card lifecycle state (§2A). Drives the single action row the card renders. */
export type SuggestedTaskState =
  | "unconnected"
  | "suggested"
  | "running"
  | "completed"
  | "failed";

export interface SuggestedTask {
  id: string;
  app: AutomationApp;
  title: string;
  /** One-line proposal/result for the card subtitle. */
  summary: string;
  state: SuggestedTaskState;
  /**
   * Instruction submitted through the normal send path when the user approves
   * the card — this is what actually runs the agent as a foreground turn. The
   * card's `title` is shown as the message's display content instead.
   */
  approvePrompt: string;
  /**
   * Scheduling instruction submitted through the normal send path when the user
   * clicks "+ Automation" on a completed card — the agent turns this into a
   * recurring automation (it calls `builtin.trigger_create`; there is no REST
   * create path, the prompt injection is the design).
   */
  automationPrompt: string;
  /** Tool name shown in the Connect CTA when `state === "unconnected"`. */
  connectLabel?: string;
}

/* ── App presentation metadata ───────────────────────────────────────── */

export interface AutomationAppMeta {
  /** Icon name from `design-system/icons`. */
  icon: string;
  /** i18n label key for the app name. */
  labelKey: string;
  /** Brand accent for the icon chip (foreground); background stays neutral. */
  accent: string;
}

const APP_META: Record<AutomationApp, AutomationAppMeta> = {
  gmail: { icon: "send", labelKey: "chat.oobe.app.gmail", accent: "#ea4335" },
  google_calendar: {
    icon: "calendar",
    labelKey: "chat.oobe.app.calendar",
    accent: "#4285f4",
  },
  google_docs: {
    icon: "file",
    labelKey: "chat.oobe.app.docs",
    accent: "#3b82f6",
  },
  slack: { icon: "chat", labelKey: "chat.oobe.app.slack", accent: "#e01e5a" },
  notion: {
    icon: "bookOpen",
    labelKey: "chat.oobe.app.notion",
    accent: "#9aa4b2",
  },
};

const FALLBACK_META: AutomationAppMeta = {
  icon: "spark",
  labelKey: "chat.oobe.app.unknown",
  accent: "#9aa4b2",
};

export function appMeta(app: AutomationApp): AutomationAppMeta {
  return APP_META[app] ?? FALLBACK_META;
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
