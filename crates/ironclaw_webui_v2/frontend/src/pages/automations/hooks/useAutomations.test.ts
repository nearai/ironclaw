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
  const invalidations = [];
  const consoleErrors = [];
  const dismissedToastIds = [];
  const toastCalls = [];
  const context = {
    AUTOMATIONS_BASE_REFETCH_MS: 30000,
    React: {
      useCallback: (callback) => callback,
      useEffect: () => {},
      useMemo: (factory) => factory(),
      useRef: (initial) => ({ current: initial }),
    },
    automationSummary: () => ({}),
    console: {
      error: (...args) => consoleErrors.push(args),
    },
    deleteAutomation: () => {},
    dismissToast: (id) => {
      if (id != null) dismissedToastIds.push(id);
    },
    globalThis: {},
    listAutomations: () => {},
    nextAutomationsRefetchDelay: () => null,
    normalizeAutomations: () => [],
    pauseAutomation: () => {},
    renameAutomation: () => {},
    resumeAutomation: () => {},
    setTimeout,
    clearTimeout,
    toast: (...args) => {
      toastCalls.push(args);
      return "toast-41";
    },
    useI18n: () => ({ lang: "en", t: (key) => `translated:${key}` }),
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
    dismissedToastIds,
    invalidations,
    mutationConfigs,
    toastCalls,
    useAutomations: context.globalThis.__testExports.useAutomations,
  };
}

test("only the latest automation action controls the error toast", () => {
  const {
    consoleErrors,
    dismissedToastIds,
    invalidations,
    mutationConfigs,
    toastCalls,
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
  assert.deepEqual(toastCalls, []);

  mutationConfigs[2].onError(
    new Error("raw backend detail"),
    { automationId: "automation-2", name: "New name" },
    secondAction
  );
  assert.deepEqual(
    JSON.parse(JSON.stringify(toastCalls)),
    [[
      "translated:automations.error.actionFailed",
      { tone: "error" },
    ]]
  );
  assert.deepEqual(consoleErrors, []);

  const thirdAction = mutationConfigs[1].onMutate("automation-3");
  assert.deepEqual(dismissedToastIds, ["toast-41"]);
  mutationConfigs[2].onSuccess(
    { updated: true },
    { automationId: "automation-2", name: "New name" },
    secondAction
  );
  assert.deepEqual(dismissedToastIds, ["toast-41"]);
  assert.equal(invalidations.length, 1);

  mutationConfigs[1].onSuccess(
    { updated: true },
    "automation-3",
    thirdAction
  );
  assert.deepEqual(dismissedToastIds, ["toast-41"]);
  assert.equal(invalidations.length, 2);
});
