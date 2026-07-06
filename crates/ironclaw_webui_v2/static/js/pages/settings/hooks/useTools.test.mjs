import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function sourceForTest(path, exportNames) {
  const source = readFileSync(new URL(path, import.meta.url), "utf8");
  const lines = [];
  for (const line of source.split("\n")) {
    if (line.startsWith("import ")) continue;
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { ${exportNames.join(", ")} };`;
}

function loadUseTools({ mutationError = null } = {}) {
  const calls = [];
  const context = {
    React: {
      useCallback: (fn) => fn,
      useState: (initial) => [
        initial,
        (updater) => calls.push({ type: "setState", updater }),
      ],
    },
    fetchTools: () => {},
    globalThis: {},
    throwIfApiFailed: (value) => value,
    updateToolPermission: () => {},
    useMutation: (options) => ({
      error: mutationError,
      mutate: (payload) => calls.push({ type: "mutate", payload }),
      reset: () => calls.push({ type: "reset" }),
      options,
    }),
    useQuery: () => ({ data: { tools: [] }, isLoading: false, error: null }),
    useQueryClient: () => ({
      setQueryData: (...args) => calls.push({ type: "setQueryData", args }),
    }),
  };

  vm.runInNewContext(sourceForTest("./useTools.js", ["useTools"]), context);

  return {
    calls,
    useTools: context.globalThis.__testExports.useTools,
  };
}

test("setPermission clears the previous mutation error before retrying save", () => {
  const { calls, useTools } = loadUseTools({ mutationError: new Error("old failure") });
  const tools = useTools();

  tools.setPermission("builtin.echo", "disabled");

  assert.equal(calls[0].type, "reset");
  assert.equal(calls[1].type, "mutate");
  assert.equal(calls[1].payload.name, "builtin.echo");
  assert.equal(calls[1].payload.state, "disabled");
});
