import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  fetchLlmProviders,
  importSettings,
  listLlmProviderModels,
  testLlmProviderConnection,
  updateSetting,
} from "../lib/settings-api.js";
import {
  API_KEY_UNCHANGED,
  isProviderConfigured,
  parseBuiltinOverrides,
  parseCustomProviders,
  providerDefaultModel,
  providerMissingReason,
} from "../lib/llm-providers.js";

export function useLlmProviders({ settings, gatewayStatus }) {
  const queryClient = useQueryClient();
  const providersQuery = useQuery({
    queryKey: ["llm-providers"],
    queryFn: fetchLlmProviders,
    staleTime: 60_000,
  });

  const builtinProviders = Array.isArray(providersQuery.data) ? providersQuery.data : [];
  const customProviders = parseCustomProviders(settings.llm_custom_providers);
  const builtinOverrides = parseBuiltinOverrides(settings.llm_builtin_overrides);
  const activeProviderId = settings.llm_backend || gatewayStatus?.llm_backend || "nearai";
  const selectedModel = settings.selected_model || gatewayStatus?.llm_model || "";
  const providers = [...builtinProviders, ...customProviders].sort((a, b) => {
    if (a.id === activeProviderId) return -1;
    if (b.id === activeProviderId) return 1;
    return (a.name || a.id).localeCompare(b.name || b.id);
  });

  const refreshSettings = () => {
    queryClient.invalidateQueries({ queryKey: ["settings-export"] });
  };

  const setActiveMutation = useMutation({
    mutationFn: async (provider) => {
      if (!isProviderConfigured(provider, builtinOverrides)) {
        const reason = providerMissingReason(provider, builtinOverrides);
        throw new Error(reason === "base_url" ? "base_url" : "api_key");
      }
      const model = providerDefaultModel(provider, builtinOverrides);
      if (!model) throw new Error("model");
      await importSettings({ settings: { llm_backend: provider.id, selected_model: model } });
      return provider;
    },
    onSuccess: refreshSettings,
  });

  const saveCustomMutation = useMutation({
    mutationFn: async ({ form, apiKey, editingProvider }) => {
      const next = [...customProviders];
      const entry = {
        ...(editingProvider || {}),
        id: form.id.trim(),
        name: form.name.trim(),
        adapter: form.adapter,
        base_url: form.baseUrl.trim(),
        default_model: form.model.trim() || undefined,
        builtin: false,
      };
      if (apiKey.trim()) {
        entry.api_key = apiKey.trim();
      } else if (editingProvider?.api_key === API_KEY_UNCHANGED) {
        entry.api_key = API_KEY_UNCHANGED;
      } else {
        delete entry.api_key;
      }

      if (editingProvider) {
        const idx = next.findIndex((provider) => provider.id === editingProvider.id);
        if (idx >= 0) next[idx] = entry;
      } else {
        next.push(entry);
      }

      await updateSetting("llm_custom_providers", next);
      if (editingProvider?.id === activeProviderId) {
        if (entry.default_model) {
          await updateSetting("selected_model", entry.default_model);
        } else {
          await updateSetting("selected_model", null);
        }
      }
      return entry;
    },
    onSuccess: refreshSettings,
  });

  const saveBuiltinMutation = useMutation({
    mutationFn: async ({ provider, form, apiKey }) => {
      const next = { ...builtinOverrides };
      const previous = next[provider.id] || {};
      const override = {};
      if (apiKey.trim()) {
        override.api_key = apiKey.trim();
      } else if (previous.api_key === API_KEY_UNCHANGED) {
        override.api_key = API_KEY_UNCHANGED;
      }
      if (form.model.trim()) override.model = form.model.trim();
      if (form.baseUrl.trim()) override.base_url = form.baseUrl.trim();
      next[provider.id] = override;

      await updateSetting("llm_builtin_overrides", next);
      if (provider.id === activeProviderId) {
        if (override.model) {
          await updateSetting("selected_model", override.model);
        } else {
          await updateSetting("selected_model", null);
        }
      }
      return provider;
    },
    onSuccess: refreshSettings,
  });

  const deleteCustomMutation = useMutation({
    mutationFn: async (provider) => {
      const next = customProviders.filter((item) => item.id !== provider.id);
      await updateSetting("llm_custom_providers", next);
      return provider;
    },
    onSuccess: refreshSettings,
  });

  return {
    providers,
    builtinProviders,
    customProviders,
    builtinOverrides,
    activeProviderId,
    selectedModel,
    isLoading: providersQuery.isLoading,
    error: providersQuery.error,
    setActiveProvider: (provider) => setActiveMutation.mutateAsync(provider),
    saveCustomProvider: (payload) => saveCustomMutation.mutateAsync(payload),
    saveBuiltinProvider: (payload) => saveBuiltinMutation.mutateAsync(payload),
    deleteCustomProvider: (provider) => deleteCustomMutation.mutateAsync(provider),
    testConnection: testLlmProviderConnection,
    listModels: listLlmProviderModels,
    isBusy:
      setActiveMutation.isPending ||
      saveCustomMutation.isPending ||
      saveBuiltinMutation.isPending ||
      deleteCustomMutation.isPending,
  };
}
