import React from "react";
import { useLlmProviders } from "./useLlmProviders";

function matchesProvider(provider, query) {
  if (!query) return true;
  const q = query.toLowerCase();
  return [provider.id, provider.name, provider.adapter, provider.base_url, provider.default_model]
    .filter(Boolean)
    .some((value) => String(value).toLowerCase().includes(q));
}

export function useProviderManagementActions({ settings, gatewayStatus, searchQuery, t }) {
  const providerState = useLlmProviders({ settings, gatewayStatus });
  const [dialogProvider, setDialogProvider] = React.useState(null);
  const [isDialogOpen, setIsDialogOpen] = React.useState(false);
  const [message, setMessage] = React.useState(null);
  const messageTimerRef = React.useRef(null);

  const showMessage = React.useCallback((tone, text) => {
    if (messageTimerRef.current) window.clearTimeout(messageTimerRef.current);
    setMessage({ tone, text });
    messageTimerRef.current = window.setTimeout(() => setMessage(null), 3500);
  }, []);

  React.useEffect(
    () => () => {
      if (messageTimerRef.current) window.clearTimeout(messageTimerRef.current);
    },
    []
  );

  const openDialog = React.useCallback((provider = null) => {
    setDialogProvider(provider);
    setIsDialogOpen(true);
  }, []);

  const handleUse = React.useCallback(
    async (provider) => {
      try {
        await providerState.setActiveProvider(provider);
        showMessage("success", t("llm.providerActivated", { name: provider.name || provider.id }));
      } catch (err) {
        if (err.message === "base_url" || err.message === "api_key" || err.message === "model") {
          openDialog(provider);
          showMessage(
            "error",
            t(err.message === "base_url" ? "llm.baseUrlRequired" : err.message === "model" ? "llm.modelRequired" : "llm.configureToUse")
          );
        } else {
          showMessage("error", err.message);
        }
      }
    },
    [openDialog, providerState, showMessage, t]
  );

  const handleSave = React.useCallback(
    async ({ form, apiKey, provider }) => {
      if (provider?.builtin) {
        await providerState.saveBuiltinProvider({ provider, form, apiKey });
        showMessage("success", t("llm.providerConfigured", { name: provider.name || provider.id }));
        return;
      }
      const saved = await providerState.saveCustomProvider({ form, apiKey, editingProvider: provider });
      showMessage("success", t(provider ? "llm.providerUpdated" : "llm.providerAdded", { name: saved.name || saved.id }));
    },
    [providerState, showMessage, t]
  );

  const handleDelete = React.useCallback(
    async (provider) => {
      if (!window.confirm(t("llm.confirmDelete", { id: provider.id }))) return;
      try {
        await providerState.deleteCustomProvider(provider);
        showMessage("success", t("llm.providerDeleted"));
      } catch (err) {
        showMessage("error", err.message);
      }
    },
    [providerState, showMessage, t]
  );

  return {
    providerState,
    dialogProvider,
    isDialogOpen,
    message,
    filteredProviders: providerState.providers.filter((provider) =>
      matchesProvider(provider, searchQuery)
    ),
    allProviderIds: providerState.providers.map((provider) => provider.id),
    openDialog,
    closeDialog: () => setIsDialogOpen(false),
    handleUse,
    handleSave,
    handleDelete,
  };
}
