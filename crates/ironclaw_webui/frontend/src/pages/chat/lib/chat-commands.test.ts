import { describe, expect, it } from "vitest";

import {
  COMMAND_RESULT_KIND,
  INITIAL_COMMAND_MENU_SELECTION,
  classifyCommandResponse,
  commandMenuMatches,
  commandMenuSelectionReducer,
  commandMenuToken,
  isIdentifierValue,
  isIsoTimestampValue,
  matchCommand,
  renderCommandResultMarkdown,
} from "./chat-commands";

// Wire shape from `GET /api/webchat/v2/commands`: `{name, title, description,
// usage}` — no aliases. The frontend matches on `name` only.
const COMMANDS = [
  {
    name: "model",
    title: "Model",
    description: "Show or switch the active LLM provider and model",
    usage: "/model",
  },
  {
    name: "status",
    title: "Status",
    description: "Show what the assistant is doing",
    usage: "/status",
  },
];

describe("matchCommand", () => {
  it("matches canonical names case-insensitively", () => {
    expect(matchCommand("/model gpt-5", COMMANDS)?.name).toBe("model");
    expect(matchCommand("  /STATUS  ", COMMANDS)?.name).toBe("status");
  });

  it("returns null for unknown commands and plain text", () => {
    expect(matchCommand("/notacommand", COMMANDS)).toBeNull();
    expect(matchCommand("hello /model", COMMANDS)).toBeNull();
    expect(matchCommand("/", COMMANDS)).toBeNull();
    expect(matchCommand("", COMMANDS)).toBeNull();
  });
});

describe("commandMenuMatches", () => {
  it("filters by the typed prefix across names", () => {
    expect(commandMenuMatches("/", COMMANDS)).toHaveLength(2);
    expect(commandMenuMatches("/mo", COMMANDS).map((c) => c.name)).toEqual([
      "model",
    ]);
    expect(commandMenuMatches("/st", COMMANDS).map((c) => c.name)).toEqual([
      "status",
    ]);
  });

  it("stops suggesting once arguments follow the command word", () => {
    expect(commandMenuMatches("/model gpt", COMMANDS)).toHaveLength(0);
    expect(commandMenuMatches("plain text", COMMANDS)).toHaveLength(0);
  });
});

describe("commandMenuToken", () => {
  it("derives the lowercased prefix while the draft is a bare command word", () => {
    expect(commandMenuToken("/mo")).toBe("mo");
    expect(commandMenuToken("  /STATUS")).toBe("status");
    expect(commandMenuToken("/")).toBe("");
  });

  it("returns null once the draft isn't (or is no longer) a bare command word", () => {
    expect(commandMenuToken("/model gpt-5")).toBeNull();
    expect(commandMenuToken("plain text")).toBeNull();
    expect(commandMenuToken("")).toBeNull();
  });
});

// Pure selection-state contract for the composer's keyboard-driven command
// menu (active row + Esc-dismissed flag). `chat-input.tsx` only dispatches
// actions and mirrors the result into useState/useRef — see its comments —
// so the wraparound/reset/dismiss math is unit-tested here without React or
// the DOM.
describe("commandMenuSelectionReducer", () => {
  it("moves the active row forward and backward with wraparound", () => {
    const forward = commandMenuSelectionReducer(
      { index: 2, dismissed: false },
      { type: "move", delta: 1, count: 3 },
    );
    expect(forward.index).toBe(0);

    const backward = commandMenuSelectionReducer(
      { index: 0, dismissed: false },
      { type: "move", delta: -1, count: 3 },
    );
    expect(backward.index).toBe(2);

    const middle = commandMenuSelectionReducer(
      { index: 0, dismissed: false },
      { type: "move", delta: 1, count: 3 },
    );
    expect(middle.index).toBe(1);
  });

  it("moving with no rows to select stays at row 0", () => {
    const state = commandMenuSelectionReducer(
      { index: 2, dismissed: false },
      { type: "move", delta: 1, count: 0 },
    );
    expect(state.index).toBe(0);
  });

  it("resets the active row and clears the dismissed flag on a filter change", () => {
    const dismissedMidList = { index: 2, dismissed: true };
    expect(commandMenuSelectionReducer(dismissedMidList, { type: "reset" })).toEqual(
      INITIAL_COMMAND_MENU_SELECTION,
    );
  });

  it("dismiss sets the flag without touching the active row", () => {
    const state = commandMenuSelectionReducer(
      { index: 1, dismissed: false },
      { type: "dismiss" },
    );
    expect(state).toEqual({ index: 1, dismissed: true });
  });

  it("select sets the active row directly (hover)", () => {
    const state = commandMenuSelectionReducer(
      { index: 0, dismissed: false },
      { type: "select", index: 2 },
    );
    expect(state).toEqual({ index: 2, dismissed: false });
  });

  it("is a no-op for an unrecognized action", () => {
    const state = { index: 1, dismissed: false };
    expect(commandMenuSelectionReducer(state, { type: "noop" })).toBe(state);
  });
});

describe("renderCommandResultMarkdown", () => {
  it("renders title, fields, and lines generically", () => {
    expect(
      renderCommandResultMarkdown({
        command: "status",
        result: {
          title: "Status",
          fields: [{ label: "State", value: "working" }],
          lines: ["since 12:00"],
        },
      }),
    ).toBe("**Status**\nState: working\nsince 12:00");
  });

  it("renders the rejection message when present", () => {
    expect(
      renderCommandResultMarkdown({
        command: "nope",
        rejection: { kind: "invalid_request", message: "Available commands:" },
      }),
    ).toBe("Available commands:");
  });
});

// Classification driving command-result.tsx's dispatch (success card /
// dropdown-echoing command list / calm denial notice). See
// `ironclaw_product/src/reborn_services/product_commands.rs`:
// `execute_product_command` always answers with exactly one of
// `result`/`rejection`, and the only rejection `kind` it emits for the
// unknown/malformed-command help text is `invalid_request` — every other
// kind reachable from command execution (in practice `access_denied`, the
// admin-only gate) is a genuine denial.
describe("classifyCommandResponse", () => {
  it("classifies a populated result as success", () => {
    expect(
      classifyCommandResponse({
        command: "status",
        result: { title: "Status", fields: [], lines: [] },
      }),
    ).toBe(COMMAND_RESULT_KIND.SUCCESS);
  });

  it("classifies an invalid_request rejection as the command list", () => {
    expect(
      classifyCommandResponse({
        command: "",
        rejection: { kind: "invalid_request", message: "Available commands:\n/status" },
      }),
    ).toBe(COMMAND_RESULT_KIND.COMMAND_LIST);
  });

  it("classifies any other rejection kind as a denial", () => {
    expect(
      classifyCommandResponse({
        command: "lifecycle_install",
        rejection: {
          kind: "access_denied",
          message: "This command requires an admin account.",
        },
      }),
    ).toBe(COMMAND_RESULT_KIND.DENIAL);
  });

  it("classifies a response with neither result nor rejection as empty", () => {
    expect(classifyCommandResponse({ command: "status" })).toBe(
      COMMAND_RESULT_KIND.EMPTY,
    );
    expect(classifyCommandResponse(null)).toBe(COMMAND_RESULT_KIND.EMPTY);
  });
});

describe("isIsoTimestampValue", () => {
  it("accepts the RFC3339-with-seconds, Z-suffixed shape the backend emits", () => {
    // Matches `DateTime::to_rfc3339_opts(SecondsFormat::Secs, true)` from
    // `execute_product_status_command`.
    expect(isIsoTimestampValue("2026-07-30T13:18:49Z")).toBe(true);
  });

  it("accepts a fractional-seconds timestamp with an explicit offset", () => {
    expect(isIsoTimestampValue("2026-07-30T13:18:49.123+00:00")).toBe(true);
  });

  it("rejects plain words, bare numbers, and non-timestamp text", () => {
    expect(isIsoTimestampValue("idle")).toBe(false);
    expect(isIsoTimestampValue("12")).toBe(false);
    expect(isIsoTimestampValue("2026-07-30")).toBe(false);
    expect(isIsoTimestampValue("")).toBe(false);
    expect(isIsoTimestampValue(undefined)).toBe(false);
  });
});

describe("isIdentifierValue", () => {
  it("accepts a run-id-shaped UUID", () => {
    expect(isIdentifierValue("1b9894d9-3f21-4a10-9abc-def012345678")).toBe(true);
  });

  it("accepts a dotted/hyphenated/slashed package id", () => {
    expect(isIdentifierValue("acme-tools/foo-bar@2.1.0")).toBe(true);
  });

  it("rejects short plain words even when a label might suggest an id", () => {
    expect(isIdentifierValue("idle")).toBe(false);
    expect(isIdentifierValue("yes")).toBe(false);
    expect(isIdentifierValue("no")).toBe(false);
  });

  it("rejects a long plain English word with no structural punctuation", () => {
    // e.g. LifecyclePublicState::as_str() -> "uninstalled" — a state WORD,
    // not an opaque identifier.
    expect(isIdentifierValue("uninstalled")).toBe(false);
  });

  it("rejects a snake_case state label even though it clears the length bar", () => {
    // "setup_needed" — underscore is deliberately excluded from the
    // identifier charset so backend State values never get monospaced.
    expect(isIdentifierValue("setup_needed")).toBe(false);
  });

  it("rejects a bare short number (e.g. a Count field)", () => {
    expect(isIdentifierValue("12")).toBe(false);
  });

  it("rejects prose containing whitespace", () => {
    expect(isIdentifierValue("No assistant activity in this conversation yet.")).toBe(
      false,
    );
  });

  it("rejects non-string values", () => {
    expect(isIdentifierValue(12)).toBe(false);
    expect(isIdentifierValue(null)).toBe(false);
    expect(isIdentifierValue(undefined)).toBe(false);
  });
});
