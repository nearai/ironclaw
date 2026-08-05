// Pure slash-command helpers for the composer. Everything is derived from the
// server inventory (`listChatCommands`) — the frontend never hardcodes a
// command name or renders a command-specific shape.

// Returns the inventory descriptor the draft's first token names (by
// canonical name), or null when the text is not a known slash command and
// should submit as an ordinary message.
export function matchCommand(text, commands) {
  const trimmed = (text || "").trim();
  if (!trimmed.startsWith("/")) return null;
  const first = trimmed.slice(1).split(/\s+/, 1)[0]?.toLowerCase();
  if (!first) return null;
  return (commands || []).find((command) => command.name === first) || null;
}

// The filter token `commandMenuMatches` derives from the draft: the
// lowercased text after "/", or null once the draft isn't (or is no longer)
// a bare command word — no leading slash, or whitespace already follows it.
// The composer also uses this to know when an Esc-dismissed menu should
// reappear: a token change (the user kept typing) always invalidates a prior
// dismissal.
export function commandMenuToken(text) {
  const trimmed = (text || "").trimStart();
  if (!trimmed.startsWith("/")) return null;
  // Once arguments follow a complete command word, stop suggesting.
  if (/\s/.test(trimmed)) return null;
  return trimmed.slice(1).toLowerCase();
}

// Inventory rows whose name starts with the draft's command prefix — the
// derived composer menu.
export function commandMenuMatches(text, commands) {
  const token = commandMenuToken(text);
  if (token === null) return [];
  return (commands || []).filter((command) => command.name.startsWith(token));
}

// --- Command-menu selection state ------------------------------------------
// Pure reducer for the composer's keyboard-driven command-menu popover: the
// active row and the Esc-dismissed flag. Kept here (not in chat-input.tsx) so
// the wraparound/reset/dismiss math is unit-testable without React or the
// DOM. The component stores the result in useState (to re-render) and
// mirrors it into a ref (to read it back synchronously inside the same
// keydown/change handlers) — the same pattern chat-input.tsx already uses
// for `text`/`textRef`.
export const INITIAL_COMMAND_MENU_SELECTION = { index: 0, dismissed: false };

export function commandMenuSelectionReducer(state, action) {
  switch (action.type) {
    case "move": {
      const { delta, count } = action;
      if (count <= 0) return { ...state, index: 0 };
      const index = ((state.index + delta) % count + count) % count;
      return { ...state, index };
    }
    case "select":
      return { ...state, index: action.index };
    case "dismiss":
      return { ...state, dismissed: true };
    // Re-filtering (a keystroke that changes the command token) drops any
    // stale row selection and un-suppresses a menu the user Esc-dismissed
    // for a different prefix.
    case "reset":
      return INITIAL_COMMAND_MENU_SELECTION;
    default:
      return state;
  }
}

// Render an executed command's response as markdown for the SYSTEM notice
// bubble: one generic shape for every command (title, label/value fields,
// plain lines) or the rejection message. Kept as the plain-text fallback
// content on every notice message (see useChat.ts's `runCommand`) — the rich
// `CommandResult` presentation (components/command-result.tsx) is additive,
// driven by the structured response stashed alongside it.
export function renderCommandResultMarkdown(response) {
  if (response?.rejection?.message) return response.rejection.message;
  const view = response?.result;
  if (!view) return `/${response?.command ?? ""} completed.`;
  const parts = [];
  if (view.title) parts.push(`**${view.title}**`);
  for (const field of view.fields || []) {
    parts.push(`${field.label}: ${field.value}`);
  }
  for (const line of view.lines || []) {
    parts.push(line);
  }
  return parts.join("\n");
}

// --- Command-result presentation (pure classification/formatting) ---------
// `components/command-result.tsx` is the presentation layer; everything it
// needs to DECIDE what to render (as opposed to how to render it) lives here
// so it's unit-testable without React or the DOM.

export const COMMAND_RESULT_KIND = Object.freeze({
  SUCCESS: "success",
  COMMAND_LIST: "commandList",
  DENIAL: "denial",
  // Defensive only: `product.commands.execute` always answers with exactly
  // one of `result`/`rejection` (see `execute_product_command` in
  // `ironclaw_assistant/src/reborn_services/product_commands.rs`) — this never
  // fires against a real backend. `CommandResult` renders nothing for it so a
  // response shaped like neither falls back to the legacy markdown notice
  // instead of an empty card.
  EMPTY: "empty",
});

// The wire's `rejection.kind` (`ProductRejectionKind`, backend enum) is a
// small fixed protocol taxonomy, not per-command metadata — this is the one
// value `product.commands.execute` uses for "unknown/malformed command, here
// is the help text" (see the same file). Every other kind reachable from
// command execution (in practice just `access_denied`, the admin-only gate)
// is a genuine denial.
const COMMAND_LIST_REJECTION_KIND = "invalid_request";

// Classify an executed command's response into the shape its presentation
// should take. `result` wins if present (defensive: the two are meant to be
// mutually exclusive on the wire).
export function classifyCommandResponse(response) {
  if (response?.result) return COMMAND_RESULT_KIND.SUCCESS;
  const rejection = response?.rejection;
  if (!rejection) return COMMAND_RESULT_KIND.EMPTY;
  return rejection.kind === COMMAND_LIST_REJECTION_KIND
    ? COMMAND_RESULT_KIND.COMMAND_LIST
    : COMMAND_RESULT_KIND.DENIAL;
}

// `isIsoTimestampValue` / `isIdentifierValue` (field-value-shape heuristics
// for the success card) live in components/command-result.tsx itself, not
// here — they're used only by that lazily-loaded module, so keeping them
// there keeps them out of the eagerly-bundled composer chunk. This module
// stays the eager, shared classification/menu logic (`chat-input.tsx` and
// `message-bubble.tsx` both need it on first paint).
