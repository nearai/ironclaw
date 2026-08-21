// @vitest-environment happy-dom

import assert from "node:assert/strict";
import React, { act } from "react";
import {
  QueryClient,
  QueryClientProvider,
  QueryObserver,
} from "@tanstack/react-query";
import { createRoot } from "react-dom/client";
import { beforeEach, test, vi } from "vitest";

const automationApi = vi.hoisted(() => ({
  deleteAutomation: vi.fn(),
  listAutomations: vi.fn(),
  pauseAutomation: vi.fn(),
  renameAutomation: vi.fn(),
  resumeAutomation: vi.fn(),
  runAutomation: vi.fn(),
}));

vi.mock("../../../lib/api", () => automationApi);
vi.mock("../../../lib/i18n", () => ({
  useI18n: () => ({
    lang: "en",
    t: (key: string) => key,
  }),
}));

import {
  createAutomationMutationConfig,
  createAutomationMutationLifecycle,
  createAutomationsQueryOptions,
  useAutomations,
} from "./useAutomations";

beforeEach(() => {
  vi.clearAllMocks();
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, reject, resolve };
}

async function renderAutomationsHook(queryClient: QueryClient) {
  let hookResult: ReturnType<typeof useAutomations> | undefined;
  function Harness() {
    hookResult = useAutomations(false);
    return null;
  }

  const container = document.createElement("div");
  const root = createRoot(container);
  await act(async () => {
    root.render(
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
        React.createElement(Harness)
      )
    );
  });
  return {
    cleanup: () => {
      act(() => root.unmount());
      queryClient.clear();
    },
    current: () => {
      assert.ok(hookResult, "useAutomations should render");
      return hookResult;
    },
  };
}

test("refresh remains active until the list and summary refetches both settle", async () => {
  const payload = { automations: [], scheduler_enabled: true };
  automationApi.listAutomations.mockResolvedValue(payload);
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  const rendered = await renderAutomationsHook(queryClient);

  try {
    await vi.waitFor(() => assert.equal(rendered.current().isRefreshing, false));
    const listRefetch = deferred<typeof payload>();
    const summaryRefetch = deferred<typeof payload>();
    automationApi.listAutomations.mockImplementation(({ includeCompleted }) =>
      includeCompleted ? summaryRefetch.promise : listRefetch.promise
    );

    let refetchPromise!: ReturnType<ReturnType<typeof useAutomations>["refetch"]>;
    act(() => {
      refetchPromise = rendered.current().refetch();
    });
    await vi.waitFor(() => assert.equal(rendered.current().isRefreshing, true));

    await act(async () => {
      listRefetch.resolve(payload);
      await listRefetch.promise;
    });
    assert.equal(
      rendered.current().isRefreshing,
      true,
      "the still-pending summary request must keep refresh active"
    );

    await act(async () => {
      summaryRefetch.resolve(payload);
      await refetchPromise;
    });
    await vi.waitFor(() => assert.equal(rendered.current().isRefreshing, false));
  } finally {
    rendered.cleanup();
  }
});

test("a background summary failure does not become the primary page error", async () => {
  const payload = { automations: [], scheduler_enabled: true };
  automationApi.listAutomations.mockImplementation(({ includeCompleted }) =>
    includeCompleted
      ? Promise.reject(new Error("summary unavailable"))
      : Promise.resolve(payload)
  );
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  const rendered = await renderAutomationsHook(queryClient);

  try {
    await vi.waitFor(() => {
      assert.equal(rendered.current().isLoading, false);
      assert.equal(rendered.current().isRefreshing, false);
      assert.equal(rendered.current().error, null);
      assert.match(
        String(rendered.current().summaryError),
        /summary unavailable/
      );
    });
    assert.deepEqual(rendered.current().automations, []);
  } finally {
    rendered.cleanup();
  }
});

test("automation filter changes retain the visible list while fetching", async () => {
  const activePayload = {
    automations: [{ automation_id: "active-automation" }],
    scheduler_enabled: true,
  };
  const completedPayload = {
    automations: [{ automation_id: "completed-automation" }],
    scheduler_enabled: true,
  };
  let resolveCompleted!: (payload: typeof completedPayload) => void;
  const completedRequest = new Promise<typeof completedPayload>((resolve) => {
    resolveCompleted = resolve;
  });
  automationApi.listAutomations.mockImplementation(({ includeCompleted }) =>
    includeCompleted ? completedRequest : Promise.resolve(activePayload)
  );

  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
    },
  });
  const observer = new QueryObserver(
    queryClient,
    createAutomationsQueryOptions(false)
  );
  const unsubscribe = observer.subscribe(() => {});

  try {
    await observer.refetch();
    assert.equal(observer.getCurrentResult().data, activePayload);

    observer.setOptions(createAutomationsQueryOptions(true));
    const filteringResult = observer.getCurrentResult();

    assert.equal(filteringResult.data, activePayload);
    assert.equal(filteringResult.isLoading, false);
    assert.equal(filteringResult.isFetching, true);
    assert.equal(filteringResult.isPlaceholderData, true);

    resolveCompleted(completedPayload);
    await vi.waitFor(() => {
      assert.equal(observer.getCurrentResult().data, completedPayload);
      assert.equal(observer.getCurrentResult().isFetching, false);
    });
  } finally {
    unsubscribe();
    queryClient.clear();
  }
});

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
