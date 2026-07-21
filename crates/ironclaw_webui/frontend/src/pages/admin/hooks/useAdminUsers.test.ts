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
      React: {},
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

  state.resetActionErrors();
  assert.deepEqual(resetCalls, [1, 2, 3, 4]);
});
