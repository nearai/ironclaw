// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../../test-support/vm-module-harness";

function createHarness() {
  const hookValues = [];
  let hookCursor = 0;
  const deleteCalls = [];
  const providerState = {
    providers: [],
    deleteCustomProvider: async (provider) => {
      deleteCalls.push(provider.id);
    },
  };
  const context = {
    React: {
      useCallback: (fn) => fn,
      useEffect: () => {},
      useRef: (initial) => {
        const index = hookCursor;
        hookCursor += 1;
        if (!(index in hookValues)) hookValues[index] = { current: initial };
        return hookValues[index];
      },
      useState: (initial) => {
        const index = hookCursor;
        hookCursor += 1;
        if (!(index in hookValues)) {
          hookValues[index] = typeof initial === "function" ? initial() : initial;
        }
        return [
          hookValues[index],
          (next) => {
            hookValues[index] =
              typeof next === "function" ? next(hookValues[index]) : next;
          },
        ];
      },
    },
    useLlmProviders: () => providerState,
    window: {
      clearTimeout: () => {},
      setTimeout: () => 1,
    },
  };
  const exports = runVmModuleForTest(
    "./useProviderManagementActions.ts",
    ["useProviderManagementActions"],
    context,
    import.meta.url
  );
  return {
    deleteCalls,
    render() {
      hookCursor = 0;
      return exports.useProviderManagementActions({
        settings: {},
        gatewayStatus: {},
        searchQuery: "",
        t: (key) => key,
      });
    },
  };
}

test("provider deletion waits for the shared dialog confirmation", async () => {
  const harness = createHarness();
  const provider = { id: "legacy-local" };
  let actions = harness.render();

  actions.handleDelete(provider);
  assert.deepEqual(harness.deleteCalls, []);

  actions = harness.render();
  assert.equal(actions.providerToDelete, provider);

  await actions.confirmDelete();
  assert.deepEqual(harness.deleteCalls, ["legacy-local"]);

  actions = harness.render();
  assert.equal(actions.providerToDelete, null);
  assert.equal(actions.message.tone, "success");
  assert.equal(actions.message.text, "llm.providerDeleted");
});

test("provider deletion can be canceled without invoking the delete action", () => {
  const harness = createHarness();
  const provider = { id: "legacy-local" };
  let actions = harness.render();

  actions.handleDelete(provider);
  actions = harness.render();
  assert.equal(actions.providerToDelete, provider);

  actions.cancelDelete();
  actions = harness.render();
  assert.equal(actions.providerToDelete, null);
  assert.deepEqual(harness.deleteCalls, []);
});
