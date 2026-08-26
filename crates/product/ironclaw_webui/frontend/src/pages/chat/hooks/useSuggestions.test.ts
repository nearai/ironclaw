// @ts-nocheck
// @vitest-environment happy-dom
//
// Two harnesses here on purpose. The sidebar test only needs to see which
// query keys a mutation option invalidates, so the light vm-module harness
// is enough. The conflict test asserts what the cache actually converges to
// after a rejected generation, which needs a real QueryClient driving real
// re-renders.
import assert from "node:assert/strict";
import React, { act } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createRoot } from "react-dom/client";
import { beforeEach, test, vi } from "vitest";

const suggestionsApi = vi.hoisted(() => ({
  dismissSuggestion: vi.fn(),
  fetchSuggestions: vi.fn(),
  generateSuggestions: vi.fn(),
  pollDelayMs: () => 1_000,
  startSuggestion: vi.fn(),
}));

vi.mock("../lib/suggestions-api", () => suggestionsApi);

import { useSuggestions } from "./useSuggestions";
import { runVmModuleForTest } from "../../../test-support/vm-module-harness";

beforeEach(() => {
  vi.clearAllMocks();
});

const READY = {
  status: "ready",
  generation_id: "gen-a",
  suggestions: [
    {
      id: "sug-1",
      title: "Triage your inbox",
      description: "Reply to routine mail.",
      suggested_prompt: "Triage my inbox.",
    },
  ],
};

async function renderSuggestionsHook(queryClient: QueryClient) {
  let hookResult: ReturnType<typeof useSuggestions> | undefined;
  function Harness() {
    hookResult = useSuggestions();
    return null;
  }

  const container = document.createElement("div");
  const root = createRoot(container);
  await act(async () => {
    root.render(
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
        React.createElement(Harness),
      ),
    );
  });
  return {
    cleanup: () => {
      act(() => root.unmount());
      queryClient.clear();
    },
    current: () => {
      assert.ok(hookResult, "useSuggestions should render");
      return hookResult;
    },
  };
}

test("a generation claimed by another client re-reads state instead of keeping the superseded set", async () => {
  // Two tabs: this one holds a `ready` set, the other one refreshes first. The
  // backend rejects this tab's generate with 409 `GenerationInProgress`. The
  // cached `ready` cards are now superseded, and nothing else would refetch
  // them — polling only runs while the cached status is `generating`, and the
  // query client disables refetch-on-focus — so the rejection has to trigger
  // the re-read itself.
  suggestionsApi.fetchSuggestions.mockResolvedValue(READY);
  const conflict = Object.assign(new Error("conflict"), { status: 409 });
  suggestionsApi.generateSuggestions.mockRejectedValue(conflict);

  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  const rendered = await renderSuggestionsHook(queryClient);

  try {
    await vi.waitFor(() => assert.equal(rendered.current().status, "ready"));
    assert.equal(suggestionsApi.fetchSuggestions.mock.calls.length, 1);

    // The winning generation is in flight by the time this tab is rejected.
    suggestionsApi.fetchSuggestions.mockResolvedValue({
      status: "generating",
      generation_id: "gen-b",
      retry_after_seconds: 1,
      suggestions: [],
    });

    await act(async () => {
      rendered.current().generate();
    });

    await vi.waitFor(() => {
      assert.ok(
        suggestionsApi.fetchSuggestions.mock.calls.length > 1,
        "the rejected generation must re-read authoritative state",
      );
      assert.equal(
        rendered.current().status,
        "generating",
        "the client converges on the generation that won",
      );
      assert.deepEqual(
        rendered.current().suggestions,
        [],
        "superseded cards must not keep rendering",
      );
    });
  } finally {
    rendered.cleanup();
  }
});

test("an accepted generation seeds the 202 body without an extra read", async () => {
  // The happy path must not regress into a refetch-on-every-generate: the 202
  // body is already the authoritative new state.
  suggestionsApi.fetchSuggestions.mockResolvedValue({
    status: "empty",
    suggestions: [],
  });
  suggestionsApi.generateSuggestions.mockResolvedValue({
    status: "generating",
    generation_id: "gen-c",
    retry_after_seconds: 1,
    suggestions: [],
  });

  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  const rendered = await renderSuggestionsHook(queryClient);

  try {
    await vi.waitFor(() => assert.equal(rendered.current().status, "empty"));
    const readsBefore = suggestionsApi.fetchSuggestions.mock.calls.length;

    await act(async () => {
      rendered.current().generate();
    });

    await vi.waitFor(() =>
      assert.equal(rendered.current().status, "generating"),
    );
    assert.equal(
      suggestionsApi.fetchSuggestions.mock.calls.length,
      readsBefore,
      "the accepted 202 body seeds the cache directly",
    );
  } finally {
    rendered.cleanup();
  }
});


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
