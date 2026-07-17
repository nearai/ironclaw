import assert from "node:assert/strict";
import { test } from "vitest";

import {
  createAutomationMutationConfig,
  createAutomationMutationLifecycle,
} from "./useAutomations";

test("automation mutation configs share an explicit latest-action lifecycle", async () => {
  const latestActionSequence = { current: 0 };
  const actionErrorToastId = { current: null as string | null };
  const dismissedToastIds: Array<string | null | undefined> = [];
  const shownToastIds: string[] = [];
  let invalidationCount = 0;

  const lifecycle = createAutomationMutationLifecycle({
    latestActionSequence,
    actionErrorToastId,
    dismissErrorToast: (id) => dismissedToastIds.push(id),
    showErrorToast: () => {
      const id = `toast-${shownToastIds.length + 1}`;
      shownToastIds.push(id);
      return id;
    },
    invalidateAutomations: () => {
      invalidationCount += 1;
    },
  });
  const pause = async (automationId: string) => ({ automationId });
  const rename = async (variables: { automationId: string; name: string }) =>
    variables;
  const pauseConfig = createAutomationMutationConfig(pause, lifecycle);
  const renameConfig = createAutomationMutationConfig(rename, lifecycle);

  assert.equal(pauseConfig.mutationFn, pause);
  assert.equal(renameConfig.mutationFn, rename);
  for (const callbackName of ["onMutate", "onError", "onSuccess"] as const) {
    assert.equal(pauseConfig[callbackName], lifecycle[callbackName]);
    assert.equal(renameConfig[callbackName], lifecycle[callbackName]);
  }

  const firstAction = await lifecycle.onMutate("automation-1");
  const secondAction = await lifecycle.onMutate({
    automationId: "automation-2",
    name: "New name",
  });
  assert.deepEqual(dismissedToastIds, []);

  lifecycle.onError(
    new Error("raw backend detail"),
    "automation-1",
    firstAction
  );
  assert.deepEqual(shownToastIds, []);

  lifecycle.onError(
    new Error("raw backend detail"),
    { automationId: "automation-2", name: "New name" },
    secondAction
  );
  assert.deepEqual(shownToastIds, ["toast-1"]);
  assert.equal(actionErrorToastId.current, "toast-1");

  // A late callback from the older action must not dismiss or overwrite the
  // toast now owned by the latest action.
  lifecycle.onError(new Error("late failure"), "automation-1", firstAction);
  lifecycle.onSuccess({ updated: true }, "automation-1", firstAction);
  assert.deepEqual(shownToastIds, ["toast-1"]);
  assert.deepEqual(dismissedToastIds, []);
  assert.equal(actionErrorToastId.current, "toast-1");
  assert.equal(invalidationCount, 1);

  const thirdAction = await lifecycle.onMutate("automation-3");
  assert.deepEqual(dismissedToastIds, ["toast-1"]);
  assert.equal(actionErrorToastId.current, null);

  lifecycle.onSuccess({ updated: true }, "automation-3", thirdAction);
  assert.deepEqual(dismissedToastIds, ["toast-1"]);
  assert.equal(invalidationCount, 2);
});
