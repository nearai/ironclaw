import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../../test-support/vm-module-harness";

function loadUseTools({ mutationError = null } = {}) {
  const calls = [];
  let mutationOptions;
  const context = {
    React: {
      useCallback: (fn) => fn,
      useRef: (initial) => ({ current: initial }),
      useState: (initial) => [
        initial,
        (updater) => calls.push({ type: "setState", updater }),
      ],
    },
    fetchTools: () => {},
    globalThis: {},
    throwIfApiFailed: (value) => value,
    updateToolPermission: () => {},
    useMutation: (options) => {
      mutationOptions = options;
      return {
        error: mutationError,
        mutate: (payload) => calls.push({ type: "mutate", payload }),
        reset: () => calls.push({ type: "reset" }),
        options,
      };
    },
    useQuery: () => ({ data: { tools: [] }, isLoading: false, error: null }),
    useQueryClient: () => ({
      setQueryData: (...args) => calls.push({ type: "setQueryData", args }),
    }),
  };

  const exports = runVmModuleForTest("./useTools.ts", ["useTools"], context, import.meta.url);

  return {
    calls,
    getMutationOptions: () => mutationOptions,
    useTools: exports.useTools,
  };
}

test("setPermission clears the previous mutation error before retrying save", () => {
  const { calls, useTools } = loadUseTools({ mutationError: new Error("old failure") });
  const tools = useTools();

  tools.setPermission("builtin.echo", "disabled");

  assert.equal(calls[0].type, "reset");
  const mutation = calls.find((call) => call.type === "mutate");
  assert.equal(mutation.payload.name, "builtin.echo");
  assert.equal(mutation.payload.state, "disabled");
});

test("setPermission immediately retains the selected permission while saving", () => {
  const { calls, useTools } = loadUseTools();
  const tools = useTools();

  tools.setPermission("builtin.echo", "disabled");

  const pendingUpdate = calls.find((call) => call.type === "setState");
  assert.ok(pendingUpdate, "expected the pending permission to update immediately");
  assert.deepEqual(JSON.parse(JSON.stringify(pendingUpdate.updater({}))), {
    "builtin.echo": { requestId: 1, state: "disabled" },
  });
  assert.equal(calls.at(-1).type, "mutate");
  assert.equal(calls.at(-1).payload.requestId, 1);
});

test("failed permission saves clear the pending selection so the server value is restored", () => {
  const { calls, getMutationOptions, useTools } = loadUseTools();
  const tools = useTools();

  tools.setPermission("builtin.echo", "disabled");
  const pendingPermission = calls.find((call) => call.type === "setState").updater({});
  const variables = calls.find((call) => call.type === "mutate").payload;

  getMutationOptions().onError(new Error("permission denied"), variables);

  const rollbackUpdate = calls.filter((call) => call.type === "setState").at(-1);
  assert.deepEqual(JSON.parse(JSON.stringify(rollbackUpdate.updater(pendingPermission))), {});
});
