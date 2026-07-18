import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../../test-support/vm-module-harness";

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

  const exports = runVmModuleForTest("./useTools.ts", ["useTools"], context, import.meta.url);

  return {
    calls,
    useTools: exports.useTools,
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
