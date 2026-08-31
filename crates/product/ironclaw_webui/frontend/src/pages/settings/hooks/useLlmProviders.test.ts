import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../../test-support/vm-module-harness";

test("provider state preserves model policy when learning is added", () => {
  const snapshot = {
    providers: [
      {
        id: "nearai",
        description: "NEAR AI",
        builtin: true,
        active: true,
        active_model: "model-a",
      },
    ],
    active: { provider_id: "nearai", model: "model-a" },
    user_model_policy: {
      provider_id: "nearai",
      workspace_default: "model-a",
      allowed_models: ["model-a", "model-b"],
    },
    learning: {
      enabled: true,
      model: "model-b",
      status: "ready",
      reason: null,
    },
  };
  const mutation = { mutateAsync: async () => {}, isPending: false };
  const context = {
    deleteLlmProvider: async () => {},
    fetchLlmProviders: async () => snapshot,
    isProviderConfigured: () => true,
    listLlmProviderModels: async () => ({ ok: true, models: [] }),
    providerDefaultModel: () => "model-a",
    providerMissingReason: () => null,
    setActiveLlm: async () => {},
    testLlmProviderConnection: async () => ({ ok: true }),
    upsertLlmProvider: async () => {},
    useMutation: () => mutation,
    useQuery: () => ({ data: snapshot, isError: false, isLoading: false, error: null }),
    useQueryClient: () => ({ invalidateQueries: () => {} }),
  };
  const exports = runVmModuleForTest(
    "./useLlmProviders.ts",
    ["useLlmProviders"],
    context,
    import.meta.url
  );

  const state = exports.useLlmProviders({ settings: {}, gatewayStatus: null, enabled: true });

  assert.deepEqual(state.userModelPolicy, snapshot.user_model_policy);
  assert.deepEqual(state.learning, snapshot.learning);
});
