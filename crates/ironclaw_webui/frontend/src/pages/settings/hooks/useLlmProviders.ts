// @ts-nocheck
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  deleteLlmProvider,
  fetchLlmProviders,
  listLlmProviderModels,
  setActiveLlm,
  testLlmProviderConnection,
  upsertLlmProvider,
} from "../lib/settings-api";
import {
  isProviderConfigured,
  providerDefaultModel,
  providerMissingReason,
} from "../lib/llm-providers";

// The v2 `/llm/providers` snapshot is the single source of truth: a unified
// provider list (built-in + operator-defined) already annotated with the active
// selection, `builtin`, and `api_key_set`. Overrides are no longer a separate
// client-side merge — the backend resolves them — so `builtinOverrides` is kept
// as an empty object purely for the shared helper signatures.
//
// Must be a stable reference: it is threaded down to the provider dialog's reset
// effect dependency array, so a fresh `{}` each render would re-run that effect
// on every parent re-render and wipe the in-progress form (the model the user
// just picked, the base URL they typed). A frozen module-level singleton keeps
// the identity constant across renders.
const EMPTY_BUILTIN_OVERRIDES = Object.freeze({});

export function useLlmProviders({ settings: _settings, gatewayStatus, enabled = true }) {
  const queryClient = useQueryClient();
  const providersQuery = useQuery({
    queryKey: ["llm-providers"],
    queryFn: fetchLlmProviders,
    enabled,
    staleTime: 60_000,
  });

  const snapshot = enabled
    ? providersQuery.data || { providers: [], active: null }
    : { providers: [], active: null };
  // If the providers query failed (e.g. 404 when the route is gated under
  // multi-user / SSO auth, or a transient 5xx / offline), we can't conclude
  // "no LLM configured" — the provider may be set operator-side at boot — so
  // callers must not treat the failure as a reason to onboard.
  const isError = enabled && providersQuery.isError;
  const builtinOverrides = EMPTY_BUILTIN_OVERRIDES;
  // Map the wire view onto the field names the components/helpers expect.
  const allProviders = (snapshot.providers || []).map((provider) => ({
    ...provider,
    name: provider.description,
    has_api_key: provider.api_key_set === true,
  }));
  // Whether the backend has a usable active provider. Prefer the persisted
  // operator snapshot, but also honor runtime/env-configured LLMs surfaced by
  // gateway status so first-run onboarding does not mask an already-live model.
  const hasActiveProvider = Boolean(snapshot.active?.provider_id || gatewayStatus?.llm_backend);
  // The honest active selection: null when nothing is configured. The grouping,
  // sort, and per-card "active" badge must key off this — otherwise a clean
  // install renders NEAR AI under the "ACTIVE" section even though no provider
  // is selected (#4857). The `nearai` fallback below is for downstream *default*
  // selection only and must never leak into active-state display.
  const activeProviderId = hasActiveProvider
    ? snapshot.active?.provider_id || gatewayStatus?.llm_backend
    : null;
  // Default provider id used when the user activates/saves without a prior
  // selection. Intentionally falls back to `nearai`; never used for grouping.
  const defaultProviderId = activeProviderId || "nearai";
  const selectedModel = snapshot.active?.model || gatewayStatus?.llm_model || "";
  const builtinProviders = allProviders.filter((provider) => provider.builtin);
  const customProviders = allProviders.filter((provider) => !provider.builtin);
  const providers = [...allProviders].sort((a, b) => {
    if (a.id === activeProviderId) return -1;
    if (b.id === activeProviderId) return 1;
    return (a.name || a.id).localeCompare(b.name || b.id);
  });

  const refresh = () => {
    queryClient.invalidateQueries({ queryKey: ["llm-providers"] });
  };

  const setActiveMutation = useMutation({
    mutationFn: async (provider) => {
      if (!isProviderConfigured(provider, builtinOverrides)) {
        const reason = providerMissingReason(provider, builtinOverrides);
        throw new Error(reason === "base_url" ? "base_url" : "api_key");
      }
      const model = providerDefaultModel(provider, builtinOverrides);
      if (!model) throw new Error("model");
      await setActiveLlm({ provider_id: provider.id, model });
      return provider;
    },
    onSuccess: refresh,
  });

  // Both custom and built-in saves go through one upsert endpoint. A built-in
  // "override" is just an overlay entry that shadows the compiled-in provider
  // by id; the backend resolves later entries last.
  const saveProviderMutation = useMutation({
    mutationFn: async ({ provider, form, apiKey, editingProvider }) => {
      const isBuiltin = Boolean(provider?.builtin);
      const id = (isBuiltin ? provider.id : form.id.trim()).trim();
      const payload = {
        id,
        name: isBuiltin ? provider.name || provider.id : form.name.trim(),
        adapter: isBuiltin ? provider.adapter : form.adapter,
        base_url: form.baseUrl.trim() || provider?.base_url || "",
        default_model: form.model.trim() || undefined,
      };
      // Only send a key when a new value was typed; otherwise leave the stored
      // one untouched (omitting the field is "unchanged" on the backend).
      if (apiKey.trim()) {
        payload.api_key = apiKey.trim();
      }
      if ((editingProvider || provider)?.id === defaultProviderId && payload.default_model) {
        payload.set_active = true;
        payload.model = payload.default_model;
      }
      await upsertLlmProvider(payload);
      return payload;
    },
    onSuccess: refresh,
  });

  const deleteCustomMutation = useMutation({
    mutationFn: async (provider) => {
      await deleteLlmProvider(provider.id);
      return provider;
    },
    onSuccess: refresh,
  });

  return {
    providers,
    builtinProviders,
    customProviders,
    builtinOverrides,
    activeProviderId,
    selectedModel,
    hasActiveProvider,
    isError,
    isLoading: providersQuery.isLoading,
    error: providersQuery.error,
    setActiveProvider: (provider) => setActiveMutation.mutateAsync(provider),
    saveCustomProvider: (payload) => saveProviderMutation.mutateAsync(payload),
    saveBuiltinProvider: (payload) => saveProviderMutation.mutateAsync(payload),
    deleteCustomProvider: (provider) => deleteCustomMutation.mutateAsync(provider),
    testConnection: testLlmProviderConnection,
    listModels: listLlmProviderModels,
    isBusy:
      setActiveMutation.isPending ||
      saveProviderMutation.isPending ||
      deleteCustomMutation.isPending,
  };
}
