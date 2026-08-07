import React from "react";
import { Badge } from "../../../design-system/badge";
import { Button } from "../../../design-system/button";
import { Card } from "../../../design-system/card";
import { ConfirmDialog } from "../../../design-system/confirm-dialog";
import { useT } from "../../../lib/i18n";
import { INFERENCE_FIELDS } from "../lib/settings-schema";
import { filterSettingsSections, matchesSearch } from "../lib/settings-search";
import { ProviderManagement } from "./provider-management";
import { SettingsGroup } from "./settings-field";
import { SettingsSearchEmpty } from "./settings-search-empty";
import { useLlmProviders } from "../hooks/useLlmProviders";

export function InferenceTab({
  settings,
  gatewayStatus,
  onSave,
  savedKeys,
  isLoading,
  searchQuery = "",
}) {
  const t = useT();
  // Source the active backend/model from the `/llm/providers` snapshot (the
  // same query the provider list below renders from) rather than the empty
  // settings/gatewayStatus stubs, which left the Model field showing "—".
  // Shares the `["llm-providers"]` react-query cache, so no extra fetch.
  const {
    activeProviderId,
    selectedModel,
    providers,
    hasActiveProvider,
    isResetting,
    resetToDefaults,
  } = useLlmProviders({ settings, gatewayStatus });
  const [resetDialogOpen, setResetDialogOpen] = React.useState(false);
  const [resetError, setResetError] = React.useState(null);
  const confirmReset = React.useCallback(async () => {
    setResetError(null);
    try {
      await resetToDefaults();
      setResetDialogOpen(false);
    } catch (error) {
      setResetError(error.message);
    }
  }, [resetToDefaults]);
  if (isLoading) {
    return (<SettingsSkeleton />);
  }

  // `activeProviderId` falls back to `nearai` for downstream defaults, so the
  // summary must gate on `hasActiveProvider` — otherwise a first-run/unconfigured
  // deployment shows `nearai` with a positive Active badge that isn't true.
  const backend = hasActiveProvider ? activeProviderId : "";
  // Match the provider card's fallback (active model → provider default_model)
  // so the summary never shows "—" while the list below shows a model.
  const activeProvider = providers.find((provider) => provider.id === activeProviderId);
  const model = hasActiveProvider
    ? selectedModel || activeProvider?.default_model || settings.selected_model || ""
    : "";
  const sections = filterSettingsSections(INFERENCE_FIELDS, settings, searchQuery, t);
  const showProviderSummary = matchesSearch(searchQuery, [
    t("inference.provider"),
    t("inference.backend"),
    backend,
    t("inference.model"),
    model,
    t("llm.resetToDefaults"),
    t("llm.confirmResetToDefaults"),
  ]);
  const showProviderManagement = matchesSearch(searchQuery, [
    t("llm.providers"),
    t("llm.providersDesc"),
    t("llm.addProvider"),
    "llm",
    "provider",
    "openai",
    "anthropic",
    "ollama",
    "near",
  ]);

  if (!showProviderSummary && !showProviderManagement && sections.length === 0) {
    return (<SettingsSearchEmpty query={searchQuery} />);
  }

  return (
    <div className="space-y-5">
      {showProviderSummary &&
      (
      <Card padding="none" className="p-4 sm:p-5">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">{t("inference.provider")}</h3>
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3">
            <div className="text-xs text-[var(--v2-text-muted)]">{t("inference.backend")}</div>
            <div className="mt-1 flex items-center gap-2">
              <span className="font-mono text-lg font-semibold text-[var(--v2-text-strong)]">{backend || t("inference.none")}</span>
              {hasActiveProvider
                ? (<Badge tone="positive" label={t("inference.active")} size="sm" />)
                : (<Badge tone="muted" label={t("llm.notConfigured")} size="sm" />)}
            </div>
          </div>
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3">
            <div className="text-xs text-[var(--v2-text-muted)]">{t("inference.model")}</div>
            <div className="mt-1 font-mono text-lg font-semibold text-[var(--v2-text-strong)]">
              {model || t("inference.none")}
            </div>
          </div>
        </div>
        <div className="mt-4 flex flex-wrap items-center justify-between gap-3 border-t border-[var(--v2-panel-border)] pt-4">
          <p className="text-sm text-[var(--v2-text-muted)]">{t("llm.resetToDefaultsDesc")}</p>
          <Button
            type="button"
            variant="danger"
            size="sm"
            onClick={() => setResetDialogOpen(true)}
          >
            {t("llm.resetToDefaults")}
          </Button>
        </div>
        {resetError ? (
          <p className="mt-3 text-sm text-[var(--v2-danger-text)]" role="status">
            {resetError}
          </p>
        ) : null}
      </Card>
      )}

      {showProviderManagement &&
      (
        <ProviderManagement
          settings={settings}
          gatewayStatus={gatewayStatus}
          searchQuery={searchQuery}
        />
      )}

      {sections.map(
        (section) =>
          (
            <SettingsGroup
              key={section.groupKey}
              groupKey={section.groupKey}
              fields={section.fields}
              settings={settings}
              onSave={onSave}
              savedKeys={savedKeys}
            />
          )
      )}
      <ConfirmDialog
        open={resetDialogOpen}
        title={t("llm.confirmResetToDefaults")}
        description={t("llm.resetToDefaultsWarning")}
        confirmLabel={t("llm.resetToDefaults")}
        isConfirming={isResetting}
        onConfirm={confirmReset}
        onCancel={() => setResetDialogOpen(false)}
      />
    </div>
  );
}

function Skeleton({ className = "" }) {
  return (
    <div
      className={"rounded animate-pulse bg-[var(--v2-surface-muted)] " + className}
    />
  );
}

function SettingsSkeleton() {
  return (
    <div className="space-y-5">
      <Card padding="md">
        <Skeleton className="mb-4 h-3 w-24" />
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <Skeleton className="h-3 w-16" />
            <Skeleton className="mt-2 h-6 w-28" />
          </div>
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <Skeleton className="h-3 w-16" />
            <Skeleton className="mt-2 h-6 w-40" />
          </div>
        </div>
      </Card>
      {[1, 2].map(
        (i) =>
          (
            <Card key={i} padding="md">
              <Skeleton className="mb-4 h-3 w-20" />
              {[1, 2, 3].map(
                (j) =>
                  (
                    <div key={j} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
                      <Skeleton className="h-4 w-32" />
                      <Skeleton className="h-9 w-36" />
                    </div>
                  )
              )}
            </Card>
          )
      )}
    </div>
  );
}
