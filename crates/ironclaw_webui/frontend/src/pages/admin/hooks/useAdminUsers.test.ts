import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../../test-support/vm-module-harness";

function loadUseAdminUsers() {
  const invalidations = [];
  const mutations = [];
  const context = {
    React: {},
    activateAdminUser: () => {},
    createAdminUser: () => {},
    deleteAdminUser: () => {},
    deleteUserSecret: () => {},
    fetchAdminUser: () => {},
    fetchAdminUsers: () => {},
    fetchUserSecrets: () => {},
    putUserSecret: () => {},
    suspendAdminUser: () => {},
    updateAdminUser: () => {},
    useMutation: (options) => {
      mutations.push(options);
      return {
        data: null,
        error: null,
        isPending: false,
        mutateAsync: () => Promise.resolve(),
        reset: () => {},
      };
    },
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
  useAdminUsers();

  return { invalidations, mutations };
}

test("role and status mutations invalidate the matching admin user detail", async () => {
  const { invalidations, mutations } = loadUseAdminUsers();

  await mutations[1].onSuccess({}, { id: "user-role", payload: { role: "admin" } });
  await mutations[3].onSuccess({}, "user-suspended");
  await mutations[4].onSuccess({}, "user-active");

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
  const { invalidations, mutations } = loadUseAdminUsers();

  await mutations[0].onSuccess({ id: "created-user" });
  await mutations[2].onSuccess({}, "deleted-user");

  assert.deepEqual(invalidations, [
    { queryKey: ["admin", "users"], exact: false },
    { queryKey: ["admin", "users"], exact: false },
  ]);
});
