import { useNavigate, useOutletContext } from "react-router";
import { useQueryClient } from "@tanstack/react-query";
import { React, html } from "../../lib/html.js";
import { useT } from "../../lib/i18n.js";
import { Badge } from "../../design-system/badge.js";
import { Button } from "../../design-system/button.js";
import { Card } from "../../design-system/card.js";
import { ProviderDialog } from "../settings/components/provider-dialog.js";
import { useProviderManagementActions } from "../settings/hooks/useProviderManagementActions.js";
import { setActiveLlm } from "../settings/lib/settings-api.js";

// First-run "choose your provider" screen. Curated providers are surfaced first,
// in this order; everything else stays reachable via Settings → Inference. The
// setup flow reuses the same ProviderDialog + actions as the Inference tab.
const FEATURED_IDS = ["openai", "anthropic", "openai_codex", "nearai", "ollama"];

export function OnboardingPage() {
  const t = useT();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { gatewayStatus } = useOutletContext();
  const actions = useProviderManagementActions({
    settings: {},
    gatewayStatus,
    searchQuery: "",
    t,
  });
  const state = actions.providerState;

  const featured = FEATURED_IDS.map((id) =>
    state.providers.find((provider) => provider.id === id)
  ).filter(Boolean);

  const handleOnboardingSave = React.useCallback(
    async ({ form, apiKey, provider }) => {
      // Persist the provider (+ any key) via the shared save path, then make it
      // the active selection and head to chat. The cold-boot reload swaps the
      // placeholder for the real provider — no restart needed.
      await actions.handleSave({ form, apiKey, provider });
      const providerId = provider?.id || form.id.trim();
      const model = form.model?.trim() || provider?.default_model || "";
      await setActiveLlm({ provider_id: providerId, model });
      await queryClient.invalidateQueries({ queryKey: ["llm-providers"] });
      actions.closeDialog();
      navigate("/chat");
    },
    [actions, navigate, queryClient]
  );

  if (state.isLoading) {
    return html`
      <div className="grid h-full place-items-center text-sm text-[var(--v2-text-muted)]">
        ${t("common.loading")}
      </div>
    `;
  }

  return html`
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex min-h-full max-w-3xl flex-col justify-center gap-6 p-6">
        <div className="text-center">
          <h1 className="text-2xl font-semibold text-[var(--v2-text-strong)]">
            ${t("onboarding.title")}
          </h1>
          <p className="mt-2 text-sm text-[var(--v2-text-muted)]">${t("onboarding.subtitle")}</p>
        </div>

        <div className="grid gap-3 sm:grid-cols-2">
          ${featured.map(
            (provider) => html`
              <${Card}
                key=${provider.id}
                className="flex items-center justify-between gap-3 p-4"
              >
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-sm font-semibold text-[var(--v2-text-strong)]">
                      ${provider.name || provider.id}
                    </span>
                    ${provider.has_api_key &&
                    html`<${Badge} tone="positive" label=${t("onboarding.ready")} size="sm" />`}
                  </div>
                  <div className="mt-1 truncate text-xs text-[var(--v2-text-muted)]">
                    ${provider.description || provider.id}
                  </div>
                </div>
                <${Button}
                  type="button"
                  size="sm"
                  variant="primary"
                  disabled=${state.isBusy}
                  onClick=${() => actions.openDialog(provider)}
                >
                  ${t("onboarding.setUp")}
                <//>
              <//>
            `
          )}
        </div>

        <div className="text-center text-xs text-[var(--v2-text-muted)]">
          ${t("onboarding.moreInSettings")}${" "}
          <button
            type="button"
            className="underline hover:text-[var(--v2-text-strong)]"
            onClick=${() => navigate("/settings/inference")}
          >
            ${t("nav.settings")}
          </button>
        </div>
      </div>

      <${ProviderDialog}
        open=${actions.isDialogOpen}
        provider=${actions.dialogProvider}
        allProviderIds=${actions.allProviderIds}
        builtinOverrides=${state.builtinOverrides}
        onClose=${actions.closeDialog}
        onSave=${handleOnboardingSave}
        onTest=${state.testConnection}
        onListModels=${state.listModels}
      />
    </div>
  `;
}
