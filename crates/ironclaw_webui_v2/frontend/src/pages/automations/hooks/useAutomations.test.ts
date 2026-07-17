// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import ts from "typescript";
import { test } from "vitest";

import { sourceTextForVmTest } from "../../../test-support/vm-module-harness";

function loadUseAutomations() {
  const source = readFileSync(new URL("./useAutomations.ts", import.meta.url), "utf8");
  const transpiled = ts.transpileModule(source, {
    compilerOptions: {
      module: ts.ModuleKind.ESNext,
      target: ts.ScriptTarget.ES2022,
    },
  }).outputText;
  const mutationConfigs = [];
  const stateUpdates = [];
  const invalidations = [];
  const consoleErrors = [];
  const context = {
    AUTOMATIONS_BASE_REFETCH_MS: 30000,
    React: {
      useCallback: (callback) => callback,
      useEffect: () => {},
      useMemo: (factory) => factory(),
      useRef: (initial) => ({ current: initial }),
      useState: (initial) => [initial, (value) => stateUpdates.push(value)],
    },
    automationSummary: () => ({}),
    console: {
      error: (...args) => consoleErrors.push(args),
    },
    deleteAutomation: () => {},
    globalThis: {},
    listAutomations: () => {},
    nextAutomationsRefetchDelay: () => null,
    normalizeAutomations: () => [],
    pauseAutomation: () => {},
    renameAutomation: () => {},
    resumeAutomation: () => {},
    setTimeout,
    clearTimeout,
    useI18n: () => ({ lang: "en", t: (key) => key }),
    useMutation: (config) => {
      mutationConfigs.push(config);
      return { isPending: false, mutate: () => {} };
    },
    useQuery: () => ({
      data: { automations: [], scheduler_enabled: true },
      error: null,
      isFetching: false,
      isLoading: false,
      refetch: () => {},
    }),
    useQueryClient: () => ({
      invalidateQueries: (query) => invalidations.push(query),
    }),
  };

  vm.runInNewContext(
    sourceTextForVmTest(transpiled, ["useAutomations"]),
    context
  );

  return {
    consoleErrors,
    invalidations,
    mutationConfigs,
    stateUpdates,
    useAutomations: context.globalThis.__testExports.useAutomations,
  };
}

test("automation action results only update the latest action state", () => {
  const {
    consoleErrors,
    invalidations,
    mutationConfigs,
    stateUpdates,
    useAutomations,
  } = loadUseAutomations();
  useAutomations();

  assert.equal(mutationConfigs.length, 4);
  for (const callbackName of ["onMutate", "onError", "onSuccess"]) {
    assert.equal(
      new Set(mutationConfigs.map((config) => config[callbackName])).size,
      1,
      `all automation mutations must share ${callbackName}`
    );
  }

  const firstAction = mutationConfigs[0].onMutate("automation-1");
  const secondAction = mutationConfigs[2].onMutate({
    automationId: "automation-2",
    name: "New name",
  });

  mutationConfigs[0].onError(
    new Error("raw backend detail"),
    "automation-1",
    firstAction
  );
  assert.deepEqual(stateUpdates, [false, false]);

  mutationConfigs[2].onError(
    new Error("raw backend detail"),
    { automationId: "automation-2", name: "New name" },
    secondAction
  );
  assert.deepEqual(stateUpdates, [false, false, true]);
  assert.deepEqual(consoleErrors, []);

  const thirdAction = mutationConfigs[1].onMutate("automation-3");
  mutationConfigs[2].onSuccess(
    { updated: true },
    { automationId: "automation-2", name: "New name" },
    secondAction
  );
  assert.deepEqual(stateUpdates, [false, false, true, false]);
  assert.equal(invalidations.length, 1);

  mutationConfigs[1].onSuccess(
    { updated: true },
    "automation-3",
    thirdAction
  );
  assert.deepEqual(stateUpdates, [false, false, true, false, false]);
  assert.equal(invalidations.length, 2);
});
