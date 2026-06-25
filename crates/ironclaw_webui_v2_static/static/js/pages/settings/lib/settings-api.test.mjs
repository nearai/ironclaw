import assert from "node:assert/strict";
import test from "node:test";

import { settingsFromOperatorConfig, toolFromConfigEntry } from "./settings-api.js";

test("settingsFromOperatorConfig maps the global auto-approve key", () => {
  const settings = settingsFromOperatorConfig({
    entries: [
      { key: "agent.auto_approve_tools", value: true },
      { key: "tool.example.run", value: { state: "ask_each_time" } },
    ],
  });

  assert.deepEqual(settings, { "agent.auto_approve_tools": true });
});

test("toolFromConfigEntry maps operator config tools for the tools tab", () => {
  assert.deepEqual(
    toolFromConfigEntry({
      key: "tool.example.run",
      mutable: true,
      source: "global",
      value: {
        name: "example.run",
        description: "Run example",
        state: "always_allow",
        default_state: "ask_each_time",
        locked: false,
        effective_source: "global",
      },
    }),
    {
      name: "example.run",
      description: "Run example",
      state: "always_allow",
      default_state: "ask_each_time",
      locked: false,
      effective_source: "global",
    }
  );
});

test("toolFromConfigEntry normalizes legacy and malformed permission values", () => {
  assert.deepEqual(
    toolFromConfigEntry({
      key: "tool.example.ask",
      mutable: false,
      source: "unknown",
      value: {
        state: "ask",
        default_state: "surprise",
      },
    }),
    {
      name: "example.ask",
      description: "",
      state: "ask_each_time",
      default_state: "ask_each_time",
      locked: true,
      effective_source: "default",
    }
  );
});
