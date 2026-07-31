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
        useEffect: () => {},
        useMemo: (factory) => factory(),
        useRef: (initial) => ({ current: initial }),
        useState: (initial) => [initial, () => {}],
      },
      useQuery: () => ({
        data: { users: [], nextCursor: null },
        dataUpdatedAt: 1,
        error: null,
      }),
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

test("admin user hook loads the next cursor once and disables polling after the attempt", async () => {
  let queryOptions;
  const fetchCalls = [];
  let rejectPage;
  const pendingPage = new Promise((_resolve, reject) => {
    rejectPage = reject;
  });
  const pageError = new Error("next page failed");
  const stateUpdates = [];
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
        useEffect: () => {},
        useMemo: (factory) => factory(),
        useRef: (initial) => ({ current: initial }),
        useState: (initial) => [initial, (value) => stateUpdates.push(value)],
      },
      useQuery: (options) => {
        queryOptions = options;
        return {
          data: {
            users: [
              { id: "user-1", display_name: "First" },
              { id: "user-2", display_name: "Second" },
            ],
            nextCursor: "cursor-1",
          },
          dataUpdatedAt: 1,
          error: null,
        };
      },
      useMutation: () => mutationState,
      useQueryClient: () => ({
        invalidateQueries: () => Promise.resolve(),
        setQueryData: () => {},
      }),
      fetchAdminUsers: async (params) => {
        fetchCalls.push(params);
        if (params.cursor) return pendingPage;
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
    JSON.parse(JSON.stringify(
      state.users.map((user) => [user.id, user.display_name]),
    )),
    [
      ["user-1", "First"],
      ["user-2", "Second"],
    ],
  );
  assert.equal(state.hasMore, true);
  assert.equal(
    queryOptions.refetchInterval({
      state: { fetchStatus: "idle" },
    }),
    10_000,
  );
  assert.equal(
    queryOptions.refetchInterval({
      state: { fetchStatus: "fetching" },
    }),
    false,
  );

  const signal = {};
  await queryOptions.queryFn({ signal });
  assert.equal(fetchCalls.length, 1);
  assert.equal(fetchCalls[0].limit, 20);
  assert.equal(fetchCalls[0].signal, signal);

  const firstRequest = state.loadMore();
  const duplicateRequest = state.loadMore();
  assert.equal(firstRequest, duplicateRequest);
  assert.equal(fetchCalls.length, 2);
  assert.equal(fetchCalls[1].limit, 20);
  assert.equal(fetchCalls[1].cursor, "cursor-1");
  assert.equal(
    queryOptions.refetchInterval({
      state: { fetchStatus: "idle" },
    }),
    false,
  );
  rejectPage(pageError);
  assert.equal(await firstRequest, null);
  assert.ok(stateUpdates.includes(pageError));
});
