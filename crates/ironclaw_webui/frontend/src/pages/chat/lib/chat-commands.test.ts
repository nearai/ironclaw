import { describe, expect, it } from "vitest";

import {
  INITIAL_COMMAND_MENU_SELECTION,
  commandMenuMatches,
  commandMenuSelectionReducer,
  commandMenuToken,
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
