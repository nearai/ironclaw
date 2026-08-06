export const INSPECTOR_TABS = ["prompt", "activity", "stats"] as const;
export type InspectorTab = (typeof INSPECTOR_TABS)[number];

export const INSPECTOR_HEALTH = {
  IDLE: "idle",
  LOADING: "loading",
  CONNECTING: "connecting",
  CONNECTED: "connected",
  RECONNECTING: "reconnecting",
  DISCONNECTED: "disconnected",
  FORBIDDEN: "forbidden",
  UNAVAILABLE: "unavailable",
} as const;

export type InspectorHealth =
  (typeof INSPECTOR_HEALTH)[keyof typeof INSPECTOR_HEALTH];

export const INSPECTOR_PREFERENCES_KEY = "ironclaw:inspector-preferences";

export interface InspectorPreferences {
  open: boolean;
  activeTab: InspectorTab;
}

const DEFAULT_PREFERENCES: InspectorPreferences = {
  open: true,
  activeTab: "prompt",
};

function browserSessionStorage(): Storage | null {
  if (typeof window === "undefined") return null;
  try {
    return window.sessionStorage;
  } catch (_) {
    return null;
  }
}

export function inspectorDebugEnabled(search = ""): boolean {
  try {
    return new URLSearchParams(search).get("debug") === "true";
  } catch (_) {
    return false;
  }
}

export function inspectorViewportMode(width: number): "mobile" | "overlay" | "sidebar" {
  if (!Number.isFinite(width) || width < 640) return "mobile";
  return width < 1280 ? "overlay" : "sidebar";
}

export function latestInspectorRunId(activeRun: unknown, messages: unknown[]): string | null {
  const current = activeRun as { runId?: unknown } | null;
  if (typeof current?.runId === "string" && current.runId) return current.runId;
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index] as { turnRunId?: unknown } | null;
    if (typeof message?.turnRunId === "string" && message.turnRunId) {
      return message.turnRunId;
    }
  }
  return null;
}

export function readInspectorPreferences(
  storage: Pick<Storage, "getItem"> | null = browserSessionStorage(),
): InspectorPreferences {
  try {
    const raw = storage?.getItem(INSPECTOR_PREFERENCES_KEY);
    if (!raw) return { ...DEFAULT_PREFERENCES };
    const parsed = JSON.parse(raw);
    const activeTab = INSPECTOR_TABS.includes(parsed?.activeTab)
      ? parsed.activeTab
      : DEFAULT_PREFERENCES.activeTab;
    return {
      open: typeof parsed?.open === "boolean" ? parsed.open : DEFAULT_PREFERENCES.open,
      activeTab,
    };
  } catch (_) {
    return { ...DEFAULT_PREFERENCES };
  }
}

export function writeInspectorPreferences(
  preferences: InspectorPreferences,
  storage: Pick<Storage, "setItem"> | null = browserSessionStorage(),
): void {
  try {
    storage?.setItem(INSPECTOR_PREFERENCES_KEY, JSON.stringify(preferences));
  } catch (_) {
    // Debug UI preferences are best effort and must never affect chat.
  }
}

interface ParsedCursor {
  streamId: string;
  sequence: number;
}

function parseCursor(value: string | null | undefined): ParsedCursor | null {
  if (!value) return null;
  const separator = value.lastIndexOf(":");
  if (separator <= 0) return null;
  const streamId = value.slice(0, separator);
  const sequence = Number(value.slice(separator + 1));
  if (!streamId || !Number.isSafeInteger(sequence) || sequence < 0) return null;
  return { streamId, sequence };
}

export function shouldAcceptInspectorCursor(
  previous: string | null,
  candidate: string | null,
): boolean {
  const next = parseCursor(candidate);
  if (!next) return false;
  const current = parseCursor(previous);
  if (!current || current.streamId !== next.streamId) return true;
  return next.sequence > current.sequence;
}

export function healthForInspectorStatus(status: number): InspectorHealth {
  if (status === 401 || status === 403) return INSPECTOR_HEALTH.FORBIDDEN;
  if (status === 404 || status === 405 || status === 501) {
    return INSPECTOR_HEALTH.UNAVAILABLE;
  }
  if (status === 408 || status === 425 || status === 429 || status >= 500) {
    return INSPECTOR_HEALTH.RECONNECTING;
  }
  return INSPECTOR_HEALTH.DISCONNECTED;
}
