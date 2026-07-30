// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../../test-support/vm-module-harness";

test("admin user hook exposes pending and error state for every management action", () => {
  const resetCalls = [];
  const mutationStates = [
    { data: null, error: null, isPending: false, variables: null },
    { error: new Error("role failed"), isPending: true, variables: { id: "user-role" } },
    { error: new Error("delete failed"), isPending: true, variables: "user-delete" },
    { error: new Error("suspend failed"), isPending: true, variables: "user-suspend" },
    { error: new Error("activate failed"), isPending: true, variables: "user-activate" },
  ].map((state, index) => ({
    data: null,
    mutateAsync: () => {},
    reset: () => resetCalls.push(index),
    ...state,
  }));
  let mutationIndex = 0;

  const exports = runVmModuleForTest(
    "./useAdminUsers.ts",
    ["useAdminUsers"],
    {
      React: {
        useCallback: (callback) => callback,
        useMemo: (factory) => factory(),
        useRef: (initial) => ({ current: initial }),
      },
      useInfiniteQuery: () => ({
        data: { pages: [] },
        error: null,
        fetchNextPage: () => Promise.resolve(),
        hasNextPage: false,
        isFetchNextPageError: false,
        isFetchingNextPage: false,
      }),
      useQuery: () => ({ data: [], error: null }),
      useMutation: () => mutationStates[mutationIndex++],
      useQueryClient: () => ({
        invalidateQueries: () => Promise.resolve(),
        setQueryData: () => {},
      }),
      fetchAdminUsers: () => {},
      fetchAdminUser: () => {},
      createAdminUser: () => {},
      updateAdminUser: () => {},
      deleteAdminUser: () => {},
      suspendAdminUser: () => {},
      activateAdminUser: () => {},
      fetchUserSecrets: () => {},
      putUserSecret: () => {},
      deleteUserSecret: () => {},
    },
    import.meta.url,
  );

  const state = exports.useAdminUsers();

  assert.equal(state.isUpdating, true);
  assert.equal(state.updateError.message, "role failed");
  assert.equal(state.updatingUserId, "user-role");
  assert.equal(state.isDeleting, true);
  assert.equal(state.deleteError.message, "delete failed");
  assert.equal(state.deletingUserId, "user-delete");
  assert.equal(state.isSuspending, true);
  assert.equal(state.suspendError.message, "suspend failed");
  assert.equal(state.suspendingUserId, "user-suspend");
  assert.equal(state.isActivating, true);
  assert.equal(state.activateError.message, "activate failed");
  assert.equal(state.activatingUserId, "user-activate");

  state.resetUpdate();
  state.resetDelete();
  state.resetSuspend();
  assert.deepEqual(resetCalls, [1, 2, 3]);

  state.resetActionErrors();
  assert.deepEqual(resetCalls, [1, 2, 3, 1, 2, 3, 4]);
});

test("admin user hook consumes cursors, deduplicates pages, and keeps polling off after load-more failure", async () => {
  let infiniteQueryOptions;
  const fetchCalls = [];
  let resolvePage;
  let fetchNextPageCalls = 0;
  const pageError = new Error("next page failed");
  const failedPageResult = {
    data: { pages: [] },
    error: pageError,
    isFetchNextPageError: true,
  };
  const pendingPage = new Promise((resolve) => {
    resolvePage = resolve;
  });
  const mutationState = {
    data: null,
    error: null,
    isPending: false,
    mutateAsync: () => {},
    reset: () => {},
    variables: null,
  };

  const exports = runVmModuleForTest(
    "./useAdminUsers.ts",
    ["useAdminUsers"],
    {
      React: {
        useCallback: (callback) => callback,
        useMemo: (factory) => factory(),
        useRef: (initial) => ({ current: initial }),
      },
      useInfiniteQuery: (options) => {
        infiniteQueryOptions = options;
        return {
          data: {
            pages: [
              {
                users: [
                  { id: "user-1", display_name: "First" },
                  { id: "user-2", display_name: "Old second" },
                ],
                nextCursor: "cursor-1",
              },
              {
                users: [
                  { id: "user-2", display_name: "Duplicate second" },
                  { id: "user-3", display_name: "Third" },
                ],
                nextCursor: "cursor-2",
              },
            ],
          },
          error: pageError,
          fetchNextPage: () => {
            fetchNextPageCalls += 1;
            return fetchNextPageCalls === 1
              ? pendingPage
              : Promise.resolve({ data: { pages: [] } });
          },
          hasNextPage: true,
          isFetchNextPageError: true,
          isFetchingNextPage: false,
        };
      },
      useQuery: () => ({ data: [], error: null }),
      useMutation: () => mutationState,
      useQueryClient: () => ({
        invalidateQueries: () => Promise.resolve(),
        setQueryData: () => {},
      }),
      fetchAdminUsers: async (params) => {
        fetchCalls.push(params);
        return { users: [], nextCursor: null };
      },
      fetchAdminUser: () => {},
      createAdminUser: () => {},
      updateAdminUser: () => {},
      deleteAdminUser: () => {},
      suspendAdminUser: () => {},
      activateAdminUser: () => {},
      fetchUserSecrets: () => {},
      putUserSecret: () => {},
      deleteUserSecret: () => {},
    },
    import.meta.url,
  );

  const state = exports.useAdminUsers();

  assert.deepEqual(
    state.users.map((user) => [user.id, user.display_name]),
    [
      ["user-1", "First"],
      ["user-2", "Old second"],
      ["user-3", "Third"],
    ],
  );
  assert.equal(state.hasMore, true);
  assert.equal(state.loadMoreError, pageError);
  assert.equal(infiniteQueryOptions.initialPageParam, null);
  assert.equal(
    infiniteQueryOptions.getNextPageParam({ nextCursor: "cursor-next" }),
    "cursor-next",
  );
  assert.equal(infiniteQueryOptions.getNextPageParam({ nextCursor: null }), undefined);
  assert.equal(
    infiniteQueryOptions.refetchInterval({
      state: { data: { pages: [{ users: [] }] }, fetchStatus: "idle" },
    }),
    10_000,
  );
  assert.equal(
    infiniteQueryOptions.refetchInterval({
      state: { data: { pages: [{ users: [] }] }, fetchStatus: "fetching" },
    }),
    false,
  );
  assert.equal(
    infiniteQueryOptions.refetchInterval({
      state: {
        data: { pages: [{ users: [] }, { users: [] }] },
        fetchStatus: "idle",
      },
    }),
    false,
  );

  const signal = {};
  await infiniteQueryOptions.queryFn({ pageParam: "cursor-requested", signal });
  assert.equal(fetchCalls.length, 1);
  assert.equal(fetchCalls[0].cursor, "cursor-requested");
  assert.equal(fetchCalls[0].signal, signal);

  const firstRequest = state.loadMore();
  const duplicateRequest = state.loadMore();
  assert.equal(firstRequest, duplicateRequest);
  assert.equal(fetchNextPageCalls, 1);
  resolvePage(failedPageResult);
  const firstResult = await firstRequest;
  assert.equal(firstResult.error, pageError);
  assert.equal(firstResult.isFetchNextPageError, true);
  assert.equal(
    infiniteQueryOptions.refetchInterval({
      state: { data: { pages: [{ users: [] }] }, fetchStatus: "idle" },
    }),
    false,
  );

  await state.loadMore();
  assert.equal(fetchNextPageCalls, 2);
});
