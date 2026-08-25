// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";
import { runVmModuleForTest } from "../../../test-support/vm-module-harness";

test("starting a suggestion refreshes its thread in the conversations sidebar", () => {
  const invalidations = [];
  const mutations = [];
  const context = {
    fetchSuggestions: async () => ({}),
    generateSuggestions: async () => ({}),
    pollDelayMs: () => false,
    startSuggestion: async () => ({}),
    dismissSuggestion: async () => ({}),
    useQuery: () => ({ data: null, isLoading: false, error: null }),
    useQueryClient: () => ({
      invalidateQueries: (request) => invalidations.push(request),
      setQueryData: () => {},
    }),
    useMutation: (options) => {
      mutations.push(options);
      return {
        mutate: () => {},
        isPending: false,
        variables: null,
        error: null,
      };
    },
  };

  const { useSuggestions } = runVmModuleForTest(
    "./useSuggestions.ts",
    ["useSuggestions"],
    context,
    import.meta.url,
  );
  useSuggestions();
  mutations[1].onSuccess();

  assert.deepEqual(
    invalidations
      .map((request) => [...request.queryKey])
      .sort(([left], [right]) => left.localeCompare(right)),
    [["suggestions"], ["threads"]],
  );
});
