import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../../test-support/vm-module-harness";

function loadUseTools({ mutationError = null } = {}) {
  const calls = [];
  const callbackDependencies = [];
  const clearedTimeouts = [];
  const activeTimeouts = new Map();
  let mutationOptions;
  let effectCleanup;
  let nextTimeoutId = 0;
  const mutate = (payload) => calls.push({ type: "mutate", payload });
  const reset = () => calls.push({ type: "reset" });
  const context = {
    React: {
      useCallback: (fn, dependencies) => {
        callbackDependencies.push(dependencies);
        return fn;
      },
      useEffect: (effect) => {
        effectCleanup = effect();
      },
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
        mutate,
        reset,
        options,
      };
    },
    useQuery: () => ({ data: { tools: [] }, isLoading: false, error: null }),
    useQueryClient: () => ({
      invalidateQueries: (...args) => calls.push({ type: "invalidateQueries", args }),
      setQueryData: (...args) => calls.push({ type: "setQueryData", args }),
    }),
    clearTimeout: (timeoutId) => {
      clearedTimeouts.push(timeoutId);
      activeTimeouts.delete(timeoutId);
    },
    setTimeout: (callback, delay) => {
      nextTimeoutId += 1;
      activeTimeouts.set(nextTimeoutId, { callback, delay });
      return nextTimeoutId;
    },
  };

  const exports = runVmModuleForTest("./useTools.ts", ["useTools"], context, import.meta.url);

  return {
    calls,
    activeTimeouts,
    callbackDependencies,
    clearedTimeouts,
    getEffectCleanup: () => effectCleanup,
    getMutationOptions: () => mutationOptions,
    mutationFunctions: { mutate, reset },
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
  const invalidation = calls.find((call) => call.type === "invalidateQueries");
  assert.deepEqual(JSON.parse(JSON.stringify(invalidation.args[0])), {
    queryKey: ["settings-tools"],
  });
});

test("a newer failed save refetches after an older successful response is ignored", () => {
  const { calls, getMutationOptions, useTools } = loadUseTools();
  const tools = useTools();

  tools.setPermission("builtin.echo", "always_allow");
  tools.setPermission("builtin.echo", "disabled");
  const mutations = calls.filter((call) => call.type === "mutate");
  const pendingUpdates = calls.filter((call) => call.type === "setState");
  const pendingPermissions = pendingUpdates.reduce(
    (state, update) => update.updater(state),
    {}
  );

  getMutationOptions().onSuccess(
    { tool: { name: "builtin.echo", state: "always_allow" } },
    mutations[0].payload
  );
  assert.equal(calls.some((call) => call.type === "setQueryData"), false);

  getMutationOptions().onError(new Error("permission denied"), mutations[1].payload);

  const rollbackUpdate = calls.filter((call) => call.type === "setState").at(-1);
  assert.deepEqual(
    JSON.parse(JSON.stringify(rollbackUpdate.updater(pendingPermissions))),
    {}
  );
  assert.equal(calls.filter((call) => call.type === "invalidateQueries").length, 2);
});

test("an older successful response cannot overwrite a newer successful save", () => {
  const { calls, getMutationOptions, useTools } = loadUseTools();
  const tools = useTools();

  tools.setPermission("builtin.echo", "always_allow");
  tools.setPermission("builtin.echo", "disabled");
  const mutations = calls.filter((call) => call.type === "mutate");

  getMutationOptions().onSuccess(
    { tool: { name: "builtin.echo", state: "disabled" } },
    mutations[1].payload
  );

  const cacheUpdates = calls.filter((call) => call.type === "setQueryData");
  assert.equal(cacheUpdates.length, 1);
  const latest = cacheUpdates[0].args[1]({
    tools: [{ name: "builtin.echo", state: "ask_each_time" }],
  });
  assert.equal(latest.tools[0].state, "disabled");

  getMutationOptions().onSuccess(
    { tool: { name: "builtin.echo", state: "always_allow" } },
    mutations[0].payload
  );

  assert.equal(calls.filter((call) => call.type === "setQueryData").length, 1);
  assert.equal(calls.filter((call) => call.type === "invalidateQueries").length, 1);
});

test("saved indicator timeouts restart per tool and are cleared on unmount", () => {
  const {
    activeTimeouts,
    calls,
    clearedTimeouts,
    getEffectCleanup,
    getMutationOptions,
    useTools,
  } = loadUseTools();
  const tools = useTools();

  tools.setPermission("builtin.echo", "always_allow");
  let mutation = calls.filter((call) => call.type === "mutate").at(-1);
  getMutationOptions().onSuccess(
    { tool: { name: "builtin.echo", state: "always_allow" } },
    mutation.payload
  );
  assert.deepEqual([...activeTimeouts.keys()], [1]);

  tools.setPermission("builtin.echo", "disabled");
  mutation = calls.filter((call) => call.type === "mutate").at(-1);
  getMutationOptions().onSuccess(
    { tool: { name: "builtin.echo", state: "disabled" } },
    mutation.payload
  );
  assert.deepEqual(clearedTimeouts, [1]);
  assert.deepEqual([...activeTimeouts.keys()], [2]);
  assert.equal(activeTimeouts.get(2).delay, 2000);

  getEffectCleanup()();
  assert.deepEqual(clearedTimeouts, [1, 2]);
  assert.equal(activeTimeouts.size, 0);
});

test("setPermission depends on stable mutation functions", () => {
  const { callbackDependencies, mutationFunctions, useTools } = loadUseTools();

  useTools();

  const dependencies = callbackDependencies.at(-1);
  assert.equal(dependencies[0], mutationFunctions.mutate);
  assert.equal(dependencies[1], mutationFunctions.reset);
});
