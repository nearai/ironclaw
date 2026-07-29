import { describe, expect, it } from "vitest";

import {
  commandMenuMatches,
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
