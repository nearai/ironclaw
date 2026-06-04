import { useNavigate, useOutletContext } from "react-router";
import { useQueryClient } from "@tanstack/react-query";
import { React, html } from "../../lib/html.js";
import { useT } from "../../lib/i18n.js";
import { Badge } from "../../design-system/badge.js";
import { Button } from "../../design-system/button.js";
import { Card } from "../../design-system/card.js";
import { ProviderDialog } from "../settings/components/provider-dialog.js";
import { useProviderManagementActions } from "../settings/hooks/useProviderManagementActions.js";
import {
  fetchLlmProviders,
  setActiveLlm,
  startCodexLogin,
  startNearaiLogin,
} from "../settings/lib/settings-api.js";

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

  const [nearaiBusy, setNearaiBusy] = React.useState(false);
  const [nearaiError, setNearaiError] = React.useState("");

  // NEAR AI browser login: open the returned auth URL, then poll the snapshot
  // until NEAR AI becomes active (the backend stores the token + reloads), then
  // head to chat.
  const handleNearaiLogin = React.useCallback(
    async (provider) => {
      setNearaiError("");
      setNearaiBusy(true);
      try {
        const { auth_url: authUrl } = await startNearaiLogin({
          provider,
          origin: window.location.origin,
        });
        window.open(authUrl, "_blank", "noopener");
        const deadline = Date.now() + 300_000;
        // eslint-disable-next-line no-constant-condition
        while (Date.now() < deadline) {
          await new Promise((resolve) => setTimeout(resolve, 2000));
          const snapshot = await fetchLlmProviders().catch(() => null);
          if (snapshot?.active?.provider_id === "nearai") {
            await queryClient.invalidateQueries({ queryKey: ["llm-providers"] });
            navigate("/chat");
            return;
          }
        }
        setNearaiError(t("onboarding.nearaiTimeout"));
      } catch (_err) {
        setNearaiError(t("onboarding.nearaiFailed"));
      } finally {
        setNearaiBusy(false);
      }
    },
    [navigate, queryClient, t]
  );

  const [codexBusy, setCodexBusy] = React.useState(false);
  const [codexError, setCodexError] = React.useState("");
  const [codexCode, setCodexCode] = React.useState(null);

  // Codex device-code login: ask the backend for a user code, show it + open the
  // verification URL, then poll the snapshot until Codex becomes active (the
  // backend completes the device flow and reloads in the background).
  const handleCodexLogin = React.useCallback(async () => {
    setCodexError("");
    setCodexCode(null);
    setCodexBusy(true);
    try {
      const { user_code: userCode, verification_uri: verificationUri } =
        await startCodexLogin();
      setCodexCode({ userCode, verificationUri });
      window.open(verificationUri, "_blank", "noopener");
      // Device codes typically expire after ~15 minutes.
      const deadline = Date.now() + 900_000;
      while (Date.now() < deadline) {
        await new Promise((resolve) => setTimeout(resolve, 3000));
        const snapshot = await fetchLlmProviders().catch(() => null);
        if (snapshot?.active?.provider_id === "openai_codex") {
          await queryClient.invalidateQueries({ queryKey: ["llm-providers"] });
          navigate("/chat");
          return;
        }
      }
      setCodexError(t("onboarding.codexTimeout"));
    } catch (_err) {
      setCodexError(t("onboarding.codexFailed"));
    } finally {
      setCodexBusy(false);
    }
  }, [navigate, queryClient, t]);

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
                ${provider.id === "nearai"
                  ? html`
                      <div className="flex shrink-0 gap-2">
                        <${Button}
                          type="button"
                          size="sm"
                          variant="secondary"
                          disabled=${nearaiBusy}
                          onClick=${() => handleNearaiLogin("github")}
                        >
                          GitHub
                        <//>
                        <${Button}
                          type="button"
                          size="sm"
                          variant="secondary"
                          disabled=${nearaiBusy}
                          onClick=${() => handleNearaiLogin("google")}
                        >
                          Google
                        <//>
                      </div>
                    `
                  : provider.id === "openai_codex"
                    ? html`
                        <${Button}
                          type="button"
                          size="sm"
                          variant="secondary"
                          disabled=${codexBusy}
                          onClick=${handleCodexLogin}
                        >
                          ${t("onboarding.codexSignIn")}
                        <//>
                      `
                    : html`
                        <${Button}
                          type="button"
                          size="sm"
                          variant="primary"
                          disabled=${state.isBusy}
                          onClick=${() => actions.openDialog(provider)}
                        >
                          ${t("onboarding.setUp")}
                        <//>
                      `}
              <//>
            `
          )}
        </div>

        ${nearaiBusy &&
        html`<div className="text-center text-xs text-[var(--v2-text-muted)]">
          ${t("onboarding.nearaiWaiting")}
        </div>`}
        ${nearaiError &&
        html`<div className="text-center text-xs text-red-300">${nearaiError}</div>`}

        ${codexCode &&
        html`<div
          className="mx-auto max-w-md rounded-lg border border-[var(--v2-border)] bg-[var(--v2-surface-raised)] p-4 text-center"
        >
          <div className="text-xs text-[var(--v2-text-muted)]">
            ${t("onboarding.codexEnterCode")}
          </div>
          <div className="mt-2 font-mono text-2xl font-semibold tracking-[0.3em] text-[var(--v2-text-strong)]">
            ${codexCode.userCode}
          </div>
          <a
            className="mt-2 inline-block text-xs underline hover:text-[var(--v2-text-strong)]"
            href=${codexCode.verificationUri}
            target="_blank"
            rel="noopener noreferrer"
          >
            ${codexCode.verificationUri}
          </a>
        </div>`}
        ${codexBusy &&
        html`<div className="text-center text-xs text-[var(--v2-text-muted)]">
          ${t("onboarding.codexWaiting")}
        </div>`}
        ${codexError &&
        html`<div className="text-center text-xs text-red-300">${codexError}</div>`}

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
