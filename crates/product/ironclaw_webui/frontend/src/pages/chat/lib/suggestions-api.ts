// OOBE suggestions surface:
// - The browser talks only to `/api/webchat/v2/suggestions*`.
// - The backend owns generation, durability, and the suggestion -> thread/run
//   binding; the browser never invents card state.
//
// Contract: `ironclaw_product_contracts` (`suggestions.list`,
// `suggestions.generate`, `suggestion.start`, `suggestion.dismiss`) — see
// docs/internal/design/oobe/VISION-RECONCILIATION.md §1.1. Generation is
// asynchronous: `generate` returns 202 with `status: "generating"` and a
// `retry_after_seconds` hint; the client polls `list` until a terminal status.

import { apiFetch, clientActionId } from "../../../lib/api";

const SUGGESTIONS_BASE = "/api/webchat/v2/suggestions";

/** Durable generation phase. Per-card lifecycle is NOT carried here — a card's
 *  run state is derived from its bound `run_id` (see `RebornSuggestion`). */
export type SuggestionGenerationStatus = "empty" | "generating" | "ready" | "failed";

/** Mirrors `RebornSuggestion`. `thread_id`/`run_id` are present once the card
 *  has been started, and persist durably so a returning user still sees the
 *  binding. Cards carry no tool/extension identity by design — connect is a
 *  separate surface (VISION-RECONCILIATION §3.1). */
export interface Suggestion {
  id: string;
  title: string;
  description: string;
  suggested_prompt: string;
  // Provider-neutral semantic icon enum + `sources`, concise human-readable
  // tool names for display. Both are required
  // in the shipped contract (PR #7694); typed optional here so the card renders
  // defensively even if a value is ever missing (see `suggestion-icons.tsx`).
  icon?: string;
  sources?: string[];
  thread_id?: string;
  run_id?: string;
}

/** Mirrors `RebornSuggestionsResponse`. */
export interface SuggestionsResponse {
  status: SuggestionGenerationStatus;
  generation_id?: string;
  retry_after_seconds?: number;
  suggestions: Suggestion[];
}

/** Mirrors `RebornSuggestionStartResponse`. */
export interface SuggestionStartResponse {
  suggestion_id: string;
  thread_id: string;
  run_id: string;
}

/** Read durable current state. Never starts work. */
export function fetchSuggestions({ signal }: { signal?: AbortSignal } = {}): Promise<SuggestionsResponse> {
  return apiFetch(SUGGESTIONS_BASE, { signal });
}

/** Claim or replay asynchronous generation. Replaying the same
 *  `client_action_id` returns the same generation rather than starting a
 *  competing run, so a double-click cannot fan out model work. */
export function generateSuggestions({
  clientActionId: clientId,
}: { clientActionId?: string } = {}): Promise<SuggestionsResponse> {
  return apiFetch(`${SUGGESTIONS_BASE}/generate`, {
    method: "POST",
    body: JSON.stringify({ client_action_id: clientId || clientActionId() }),
  });
}

/** Create (or replay) the thread/run bound to one suggestion. The backend
 *  submits the turn through the normal ProductSurface path — the browser does
 *  not inject the prompt itself. */
export function startSuggestion(suggestionId: string): Promise<SuggestionStartResponse> {
  return apiFetch(`${SUGGESTIONS_BASE}/${encodeURIComponent(suggestionId)}/start`, {
    method: "POST",
  });
}

/** Soft-dismiss the current card; future list responses omit it. */
export function dismissSuggestion(suggestionId: string) {
  return apiFetch(`${SUGGESTIONS_BASE}/${encodeURIComponent(suggestionId)}`, {
    method: "DELETE",
  });
}

/** Human-readable provenance for a card's `sources`. Sources are already
 *  human-readable names from the backend, so this only joins them. */
export function formatSources(sources: string[] | null | undefined): string {
  const list = (sources || []).filter((s) => typeof s === "string" && s.trim());
  if (list.length === 0) return "";
  if (list.length === 1) return list[0];
  return `${list.slice(0, -1).join(", ")} & ${list[list.length - 1]}`;
}

/** Poll delay for a `generating` response. The backend's own
 *  `retry_after_seconds` hint wins; the floor keeps a missing/0 hint from
 *  becoming a hot loop, and the ceiling keeps a hostile value from stalling
 *  the surface indefinitely. */
export function pollDelayMs(response: SuggestionsResponse | null | undefined): number {
  const hint = Number(response?.retry_after_seconds);
  const seconds = Number.isFinite(hint) && hint > 0 ? hint : 1;
  return Math.min(Math.max(seconds, 1), 30) * 1000;
}
