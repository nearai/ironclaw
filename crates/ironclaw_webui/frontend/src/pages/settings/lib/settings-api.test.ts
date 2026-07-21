import assert from "node:assert/strict";
import { test, vi } from "vitest";

import {
  settingsFromOperatorConfig,
  toolFromConfigEntry,
  updateToolPermission,
} from "./settings-api";

function toolPermissionResponse({
  name = "builtin.echo",
  state = "disabled",
  defaultState = "ask_each_time",
  source = "override",
  mutable = true,
} = {}) {
  return {
    entry: {
      key: `tool.${name}`,
      value: {
        name,
        state,
        default_state: defaultState,
        locked: !mutable,
        effective_source: source,
      },
      mutable,
      source,
    },
  };
}

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

test("updateToolPermission aborts a lost request after the bounded save timeout", async () => {
  vi.useFakeTimers();
  let requestOptions;
  vi.stubGlobal("sessionStorage", {
    getItem: () => "",
    removeItem: () => {},
    setItem: () => {},
  });
  vi.stubGlobal(
    "fetch",
    (_path, options) =>
      new Promise((_resolve, reject) => {
        requestOptions = options;
        options.signal.addEventListener(
          "abort",
          () => reject(new Error("permission save aborted")),
          { once: true }
        );
      })
  );

  try {
    const update = updateToolPermission("builtin.echo", "disabled");
    const rejected = assert.rejects(update, /permission save aborted/);

    await vi.advanceTimersByTimeAsync(30_000);
    await rejected;

    assert.equal(requestOptions.signal.aborted, true);
    assert.equal(JSON.parse(requestOptions.body).state, "disabled");
  } finally {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  }
});

test("updateToolPermission rejects missing or malformed persisted entries", async () => {
  const responses = [{}, toolPermissionResponse({ state: "unexpected" })];
  vi.stubGlobal("sessionStorage", {
    getItem: () => "",
    removeItem: () => {},
    setItem: () => {},
  });
  vi.stubGlobal(
    "fetch",
    async () =>
      new Response(JSON.stringify(responses.shift()), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      })
  );

  try {
    await assert.rejects(
      updateToolPermission("builtin.echo", "disabled"),
      /missing a valid persisted tool entry/
    );
    await assert.rejects(
      updateToolPermission("builtin.echo", "disabled"),
      /missing a valid persisted tool entry/
    );
  } finally {
    vi.unstubAllGlobals();
  }
});

test("updateToolPermission requires the persisted entry to confirm the requested state", async () => {
  vi.stubGlobal("sessionStorage", {
    getItem: () => "",
    removeItem: () => {},
    setItem: () => {},
  });
  vi.stubGlobal(
    "fetch",
    async () =>
      new Response(JSON.stringify(toolPermissionResponse({ state: "ask_each_time" })), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      })
  );

  try {
    await assert.rejects(
      updateToolPermission("builtin.echo", "disabled"),
      /did not confirm the requested tool state/
    );
  } finally {
    vi.unstubAllGlobals();
  }
});

test("updateToolPermission accepts canonical override and default responses", async () => {
  const responses = [
    toolPermissionResponse(),
    toolPermissionResponse({ state: "ask_each_time", source: "global" }),
  ];
  vi.stubGlobal("sessionStorage", {
    getItem: () => "",
    removeItem: () => {},
    setItem: () => {},
  });
  vi.stubGlobal(
    "fetch",
    async () =>
      new Response(JSON.stringify(responses.shift()), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      })
  );

  try {
    const override = await updateToolPermission("builtin.echo", "disabled");
    assert.equal(override.success, true);
    assert.equal(override.tool.state, "disabled");

    const inherited = await updateToolPermission("builtin.echo", "default");
    assert.equal(inherited.success, true);
    assert.equal(inherited.tool.state, "ask_each_time");
    assert.equal(inherited.tool.effective_source, "global");
  } finally {
    vi.unstubAllGlobals();
  }
});
