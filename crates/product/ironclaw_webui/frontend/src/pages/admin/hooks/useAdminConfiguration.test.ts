import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../../test-support/vm-module-harness";

test("a revision conflict refetches configuration so the preserved dirty values can retry", async () => {
  const invalidations = [];
  let mutationOptions;
  const exports = runVmModuleForTest(
    "./useAdminConfiguration.ts",
    ["useAdminConfiguration"],
    {
      fetchExtensionAdminConfiguration: () => {},
      replaceExtensionAdminConfiguration: () => {},
      useMutation: (options) => {
        mutationOptions = options;
        return {
          error: null,
          isPending: false,
          mutateAsync: () => {},
          reset: () => {},
          variables: null,
        };
      },
      useQuery: () => ({ data: [], error: null, isLoading: false }),
      useQueryClient: () => ({
        invalidateQueries: (request) => {
          invalidations.push(request);
          return Promise.resolve();
        },
        setQueryData: () => {},
      }),
    },
    import.meta.url,
  );

  exports.useAdminConfiguration();
  await mutationOptions.onError({ status: 409 });
  assert.deepEqual(JSON.parse(JSON.stringify(invalidations)), [
    { queryKey: ["admin", "extension-configuration"] },
  ]);

  await mutationOptions.onError({ status: 503 });
  assert.equal(invalidations.length, 1);
});
