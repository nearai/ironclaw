import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../../test-support/vm-module-harness";

function loadUseAdminUsers() {
  const invalidations = [];
  const context = {
    React: {},
    activateAdminUser: (id) => ({ id, status: "active" }),
    createAdminUser: (payload) => ({ id: "created-user", ...payload }),
    deleteAdminUser: () => ({}),
    deleteUserSecret: () => {},
    fetchAdminUser: () => {},
    fetchAdminUsers: () => {},
    fetchUserSecrets: () => {},
    putUserSecret: () => {},
    suspendAdminUser: (id) => ({ id, status: "suspended" }),
    updateAdminUser: (id, payload) => ({ id, ...payload }),
    useMutation: (options) => ({
      data: null,
      error: null,
      isPending: false,
      mutateAsync: async (variables) => {
        const data = await options.mutationFn(variables);
        await options.onSuccess?.(data, variables);
        return data;
      },
      reset: () => {},
    }),
    useQuery: () => ({ data: { users: [] }, error: null }),
    useQueryClient: () => ({
      invalidateQueries: (request) => {
        invalidations.push({
          queryKey: [...request.queryKey],
          exact: request.exact === true,
        });
        return Promise.resolve();
      },
    }),
  };

  const { useAdminUsers } = runVmModuleForTest(
    "./useAdminUsers.ts",
    ["useAdminUsers"],
    context,
    import.meta.url,
  );
  const adminUsers = useAdminUsers();

  return { adminUsers, invalidations };
}

test("role and status mutations invalidate the matching admin user detail", async () => {
  const { adminUsers, invalidations } = loadUseAdminUsers();

  await adminUsers.updateUser("user-role", { role: "admin" });
  await adminUsers.suspendUser("user-suspended");
  await adminUsers.activateUser("user-active");

  assert.deepEqual(invalidations, [
    { queryKey: ["admin", "users"], exact: false },
    { queryKey: ["admin", "user", "user-role"], exact: true },
    { queryKey: ["admin", "users"], exact: false },
    { queryKey: ["admin", "user", "user-suspended"], exact: true },
    { queryKey: ["admin", "users"], exact: false },
    { queryKey: ["admin", "user", "user-active"], exact: true },
  ]);
});

test("create and delete mutations only invalidate the admin user list", async () => {
  const { adminUsers, invalidations } = loadUseAdminUsers();

  await adminUsers.createUser({ display_name: "Created User" });
  await adminUsers.deleteUser("deleted-user");

  assert.deepEqual(invalidations, [
    { queryKey: ["admin", "users"], exact: false },
    { queryKey: ["admin", "users"], exact: false },
  ]);
});

test("a missing user id never invalidates an admin user detail query", async () => {
  const { adminUsers, invalidations } = loadUseAdminUsers();

  await adminUsers.suspendUser();

  assert.deepEqual(invalidations, [
    { queryKey: ["admin", "users"], exact: false },
  ]);
});
