import { Button } from "../../../design-system/button.js";
import { Card } from "../../../design-system/card.js";
import { Icon } from "../../../design-system/icons.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { SettingsSearchEmpty } from "./settings-search-empty.js";
import { ProviderCard } from "./provider-card.js";
import { ProviderDialog } from "./provider-dialog.js";
import { useProviderManagementActions } from "../hooks/useProviderManagementActions.js";

export function ProviderManagement({ settings, gatewayStatus, searchQuery = "" }) {
  const t = useT();
  const actions = useProviderManagementActions({ settings, gatewayStatus, searchQuery, t });
  const state = actions.providerState;

  if (searchQuery && actions.filteredProviders.length === 0) {
    return html`<${SettingsSearchEmpty} query=${searchQuery} />`;
  }

  return html`
    <${Card} className="p-4 sm:p-6">
      <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
            ${t("llm.providers")}
          </h3>
          <p className="mt-1 text-sm text-[var(--v2-text-muted)]">${t("llm.providersDesc")}</p>
        </div>
        <${Button} type="button" variant="secondary" size="sm" className="gap-2" onClick=${() => actions.openDialog(null)}>
          <${Icon} name="plus" className="h-3.5 w-3.5" />
          ${t("llm.addProvider")}
        <//>
      </div>

      ${actions.message &&
      html`
        <div
          className=${[
            "mb-4 rounded-md border px-3 py-2 text-sm",
            actions.message.tone === "error"
              ? "border-red-400/30 bg-red-500/10 text-red-200"
              : "border-mint/30 bg-mint/10 text-mint",
          ].join(" ")}
          role="status"
        >
          ${actions.message.text}
        </div>
      `}

      ${state.isLoading
        ? html`<div className="text-sm text-[var(--v2-text-muted)]">${t("common.loading")}</div>`
        : state.error
        ? html`<div className="text-sm text-red-200">${t("error.loadFailed", { what: t("llm.providers"), message: state.error.message })}</div>`
        : html`
            <div className="space-y-3">
              ${actions.filteredProviders.map(
                (provider) => html`
                  <${ProviderCard}
                    key=${provider.id}
                    provider=${provider}
                    activeProviderId=${state.activeProviderId}
                    selectedModel=${state.selectedModel}
                    builtinOverrides=${state.builtinOverrides}
                    isBusy=${state.isBusy}
                    onUse=${actions.handleUse}
                    onConfigure=${actions.openDialog}
                    onDelete=${actions.handleDelete}
                  />
                `
              )}
            </div>
          `}

      <${ProviderDialog}
        open=${actions.isDialogOpen}
        provider=${actions.dialogProvider}
        allProviderIds=${actions.allProviderIds}
        builtinOverrides=${state.builtinOverrides}
        onClose=${actions.closeDialog}
        onSave=${actions.handleSave}
        onTest=${state.testConnection}
        onListModels=${state.listModels}
      />
    <//>
  `;
}
