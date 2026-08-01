/**
 * Agent mode — the per-user autonomy setting the composer mode pill reads and
 * writes. Four levels, ordered by how much the agent may do without asking:
 *
 *   suggest — always require approval before any action (default, safest).
 *   plan    — draft a combined plan of productivity tasks, then wait for approval.
 *   auto    — auto-run user-approved task *types* (email triage, invite accepts,
 *             doc insights) without a per-action prompt; still surfaces results.
 *   bypass  — full automation, no approvals at all.
 *
 * SEAM (design prototype): the mode is persisted to scoped localStorage so the
 * pill is functional in the running app with no backend. The intended durable
 * home is a user setting on the session — read from
 * `GET /api/webchat/v2/settings/agent-mode` and written via
 * `POST /api/webchat/v2/settings/agent-mode` — mirrored onto
 * `session.features`/settings the same way `global_auto_approve` already is.
 * When that lands, swap the localStorage read/write below for the API + session
 * hydration; the `AgentMode` type and `useAgentMode()` surface stay identical.
 */
import React from "react";
import { authScope } from "../../../lib/auth-scope";

export const AGENT_MODES = Object.freeze({
  SUGGEST: "suggest",
  PLAN: "plan",
  AUTO: "auto",
  BYPASS: "bypass",
} as const);

export type AgentMode = (typeof AGENT_MODES)[keyof typeof AGENT_MODES];

export const AGENT_MODE_ORDER: AgentMode[] = [
  AGENT_MODES.SUGGEST,
  AGENT_MODES.PLAN,
  AGENT_MODES.AUTO,
  AGENT_MODES.BYPASS,
];

export const DEFAULT_AGENT_MODE: AgentMode = AGENT_MODES.SUGGEST;

const STORAGE_PREFIX = "ironclaw:v2:agent-mode:";

function isAgentMode(value: unknown): value is AgentMode {
  return (
    value === AGENT_MODES.SUGGEST ||
    value === AGENT_MODES.PLAN ||
    value === AGENT_MODES.AUTO ||
    value === AGENT_MODES.BYPASS
  );
}

function storageKey(): string {
  return `${STORAGE_PREFIX}${authScope()}`;
}

/** Read the persisted mode for the active identity (falls back to the default). */
export function getAgentMode(): AgentMode {
  try {
    const raw = window.localStorage.getItem(storageKey());
    return isAgentMode(raw) ? raw : DEFAULT_AGENT_MODE;
  } catch {
    return DEFAULT_AGENT_MODE;
  }
}

const listeners = new Set<() => void>();

function notify(): void {
  for (const listener of Array.from(listeners)) listener();
}

/** Persist the mode for the active identity and notify subscribers. */
export function setAgentMode(mode: AgentMode): void {
  if (!isAgentMode(mode)) return;
  try {
    window.localStorage.setItem(storageKey(), mode);
  } catch {
    // Non-fatal: an unavailable localStorage (private mode / quota) still
    // updates the in-memory subscribers below so the pill reflects the choice
    // for the current session.
  }
  notify();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * React binding for the agent mode. Returns the current mode plus a setter.
 * Re-reads on identity change (the storage key is scope-derived) via the
 * `authScope()` dependency so switching accounts does not leak a mode.
 */
export function useAgentMode(): [AgentMode, (mode: AgentMode) => void] {
  const scope = authScope();
  const [mode, setMode] = React.useState<AgentMode>(() => getAgentMode());

  React.useEffect(() => {
    setMode(getAgentMode());
    return subscribe(() => setMode(getAgentMode()));
  }, [scope]);

  const update = React.useCallback((next: AgentMode) => {
    setAgentMode(next);
  }, []);

  return [mode, update];
}
