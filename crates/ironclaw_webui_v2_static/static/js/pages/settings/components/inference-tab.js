import { html } from "../../../lib/html.js";
import { Badge } from "../../../design-system/badge.js";
import { Card } from "../../../design-system/card.js";
import { useT } from "../../../lib/i18n.js";
import { INFERENCE_FIELDS } from "../lib/settings-schema.js";
import { filterSettingsSections, matchesSearch } from "../lib/settings-search.js";
import { ProviderManagement } from "./provider-management.js";
import { SettingsGroup } from "./settings-field.js";
import { SettingsSearchEmpty } from "./settings-search-empty.js";
import { useLlmProviders } from "../hooks/useLlmProviders.js";

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
  const { activeProviderId, selectedModel, providers, hasActiveProvider } = useLlmProviders({
    settings,
    gatewayStatus,
  });
  if (isLoading) {
    return html`<${SettingsSkeleton} />`;
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
    return html`<${SettingsSearchEmpty} query=${searchQuery} />`;
  }

  return html`
    <div className="space-y-5">
      ${showProviderSummary &&
      html`
      <${Card} padding="none" className="p-4 sm:p-5">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">${t("inference.provider")}</h3>
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3">
            <div className="text-xs text-[var(--v2-text-muted)]">${t("inference.backend")}</div>
            <div className="mt-1 flex items-center gap-2">
              <span className="font-mono text-lg font-semibold text-[var(--v2-text-strong)]">${backend || t("inference.none")}</span>
              ${hasActiveProvider
                ? html`<${Badge} tone="positive" label=${t("inference.active")} size="sm" />`
                : html`<${Badge} tone="muted" label=${t("llm.notConfigured")} size="sm" />`}
            </div>
          </div>
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3">
            <div className="text-xs text-[var(--v2-text-muted)]">${t("inference.model")}</div>
            <div className="mt-1 font-mono text-lg font-semibold text-[var(--v2-text-strong)]">
              ${model || t("inference.none")}
            </div>
          </div>
        </div>
      <//>
      `}

      ${showProviderManagement &&
      html`
        <${ProviderManagement}
          settings=${settings}
          gatewayStatus=${gatewayStatus}
          searchQuery=${searchQuery}
        />
      `}

      ${sections.map(
        (section) =>
          html`
            <${SettingsGroup}
              key=${section.groupKey}
              groupKey=${section.groupKey}
              fields=${section.fields}
              settings=${settings}
              onSave=${onSave}
              savedKeys=${savedKeys}
            />
          `
      )}
    </div>
  `;
}

function Skeleton({ className = "" }) {
  return html`
    <div
      className=${"rounded animate-pulse bg-[var(--v2-surface-muted)] " + className}
    />
  `;
}

function SettingsSkeleton() {
  return html`
    <div className="space-y-5">
      <${Card} padding="md">
        <${Skeleton} className="mb-4 h-3 w-24" />
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <${Skeleton} className="h-3 w-16" />
            <${Skeleton} className="mt-2 h-6 w-28" />
          </div>
          <div className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
            <${Skeleton} className="h-3 w-16" />
            <${Skeleton} className="mt-2 h-6 w-40" />
          </div>
        </div>
      <//>
      ${[1, 2].map(
        (i) =>
          html`
            <${Card} key=${i} padding="md">
              <${Skeleton} className="mb-4 h-3 w-20" />
              ${[1, 2, 3].map(
                (j) =>
                  html`
                    <div key=${j} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0">
                      <${Skeleton} className="h-4 w-32" />
                      <${Skeleton} className="h-9 w-36" />
                    </div>
                  `
              )}
            <//>
          `
      )}
    </div>
  `;
}
