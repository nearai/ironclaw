import { Navigate, useNavigate, useOutletContext } from "react-router";
import { useQueryClient } from "@tanstack/react-query";
import { React, html } from "../../lib/html.js";
import { useT } from "../../lib/i18n.js";
import { Badge } from "../../design-system/badge.js";
import { Button } from "../../design-system/button.js";
import { Card } from "../../design-system/card.js";
import { Icon } from "../../design-system/icons.js";
import { ProviderDialog } from "../settings/components/provider-dialog.js";
import { ProviderLoginStatus } from "../settings/components/provider-login-status.js";
import { useProviderManagementActions } from "../settings/hooks/useProviderManagementActions.js";
import { useProviderLogin } from "../settings/hooks/useProviderLogin.js";
import { isProviderConfigured } from "../settings/lib/llm-providers.js";
import { setActiveLlm } from "../settings/lib/settings-api.js";
import { ProviderLogo } from "./provider-logos.js";

// First-run "choose your provider" list. Curated providers are surfaced in this
// order; everything else stays reachable via Settings → Inference. `auth` is the
// editorial signal for how a row authenticates (browser login vs device code vs
// API key) — the day the backend exposes a credential-kind discriminator on the
// provider view, this list can key off that instead.
const FEATURED = [
  { id: "nearai", auth: "nearai", nameKey: "onboarding.providerNearai", descKey: "onboarding.providerNearaiDesc" },
  { id: "openai_codex", auth: "codex", nameKey: "onboarding.providerCodex", descKey: "onboarding.providerCodexDesc" },
  { id: "openai", auth: "key", nameKey: "onboarding.providerOpenai", descKey: "onboarding.providerOpenaiDesc" },
  { id: "anthropic", auth: "key", nameKey: "onboarding.providerAnthropic", descKey: "onboarding.providerAnthropicDesc" },
  { id: "ollama", auth: "key", nameKey: "onboarding.providerOllama", descKey: "onboarding.providerOllamaDesc" },
];

function NearAiSetupMenu({ provider, isBusy, login, t, onSetUp }) {
  const [open, setOpen] = React.useState(false);
  const ref = React.useRef(null);
  const setupBusy = isBusy || login.nearaiBusy;

  React.useEffect(() => {
    if (!open) return undefined;
    const onDoc = (event) => {
      if (ref.current && !ref.current.contains(event.target)) setOpen(false);
    };
    const onKeyDown = (event) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  const menuItems = [
    {
      id: "api-key",
      label: t("llm.addApiKey"),
      disabled: isBusy,
      run: () => onSetUp(provider),
    },
    {
      id: "near-wallet",
      label: t("onboarding.nearWallet"),
      disabled: login.nearaiBusy,
      run: login.startNearaiWallet,
    },
    {
      id: "github",
      label: "GitHub",
      disabled: login.nearaiBusy,
      run: () => login.startNearai("github"),
    },
    {
      id: "google",
      label: "Google",
      disabled: login.nearaiBusy,
      run: () => login.startNearai("google"),
    },
  ];

  return html`
    <div ref=${ref} className="relative shrink-0">
      <${Button}
        type="button"
        variant="primary"
        size="sm"
        className="gap-1.5"
        aria-haspopup="true"
        aria-expanded=${open ? "true" : "false"}
        disabled=${setupBusy}
        onClick=${() => setOpen((v) => !v)}
      >
        ${t("onboarding.setUp")}
        <${Icon} name="chevron" className="h-3.5 w-3.5" />
      <//>
      ${open &&
      html`
        <div
          role="menu"
          className="absolute right-0 top-10 z-20 min-w-[176px] rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] p-1 shadow-[0_20px_40px_-20px_rgba(0,0,0,0.7)]"
        >
          ${menuItems.map(
            (item) => html`
              <button
                key=${item.id}
                type="button"
                role="menuitem"
                disabled=${item.disabled}
                onClick=${() => {
                  setOpen(false);
                  item.run();
                }}
                className="flex w-full items-center rounded-[7px] px-2.5 py-1.5 text-left text-[13px] text-[var(--v2-text)] hover:bg-[var(--v2-surface-soft)] disabled:cursor-not-allowed disabled:opacity-50"
              >
                ${item.label}
              </button>
            `
          )}
        </div>
      `}
    </div>
  `;
}

// One provider row: logo + name/subtitle on the left, the auth action(s) on the
// right. Stacks vertically on mobile (actions wrap onto their own line) and sits
// on a single line from `sm` up.
function FeaturedProviderRow({ entry, provider, configured, isBusy, login, t, onUse, onSetUp }) {
  const name = t(entry.nameKey);

  // Login-based providers keep their setup entrypoints visible because they
  // activate through provider-owned auth flows rather than a generic "Use".
  let actions;
  if (entry.auth === "nearai") {
    actions = html`<${NearAiSetupMenu} provider=${provider} isBusy=${isBusy} login=${login} t=${t} onSetUp=${onSetUp} />`;
  } else if (entry.auth === "codex") {
    actions = html`
      <${Button} type="button" variant="secondary" size="sm" disabled=${login.codexBusy} onClick=${login.startCodex}>
        ${t("onboarding.signIn")}
      <//>
    `;
  } else if (configured) {
    actions = html`<${Button} type="button" variant="primary" size="sm" disabled=${isBusy} onClick=${() => onUse(provider)}>
      ${t("llm.use")}
    <//>`;
  } else {
    actions = html`<${Button} type="button" variant="primary" size="sm" disabled=${isBusy} onClick=${() => onSetUp(provider)}>
      ${t("onboarding.setUp")}
    <//>`;
  }

  return html`
    <${Card} className="flex flex-col gap-3 p-4 sm:flex-row sm:items-center sm:gap-4">
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <${ProviderLogo} id=${entry.id} name=${name} />
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-semibold text-[var(--v2-text-strong)]">${name}</span>
            ${configured &&
            html`<${Badge} tone="positive" label=${t("onboarding.ready")} size="sm" />`}
          </div>
          <div className="mt-0.5 truncate text-xs text-[var(--v2-text-muted)]">${t(entry.descKey)}</div>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap gap-2 sm:justify-end">${actions}</div>
    <//>
  `;
}

export function OnboardingPage() {
  const { isAdmin = false, isChecking = false } = useOutletContext();
  if (isChecking) return null;
  if (!isAdmin) {
    return html`<${Navigate} to="/chat" replace />`;
  }
  return html`<${OperatorOnboardingPage} />`;
}

function OperatorOnboardingPage() {
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

  const featured = FEATURED.map((entry) => ({
    entry,
    provider: state.providers.find((provider) => provider.id === entry.id),
  })).filter((row) => row.provider);

  // NEAR AI + Codex login share the same backend flows as the Inference tab; on
  // success here we head straight to chat (the snapshot refresh swaps in the
  // now-active provider).
  const navigateToChat = React.useCallback(() => navigate("/chat"), [navigate]);
  const login = useProviderLogin({ onSuccess: navigateToChat });

  // Make an already-configured provider (env key present, local Ollama, etc.)
  // the active selection and head to chat without opening the dialog.
  const handleUse = React.useCallback(
    async (provider) => {
      const model = provider.active_model || provider.default_model || "";
      await setActiveLlm({ provider_id: provider.id, model });
      await queryClient.invalidateQueries({ queryKey: ["llm-providers"] });
      navigate("/chat");
    },
    [navigate, queryClient]
  );

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
      <div className="mx-auto flex min-h-full max-w-2xl flex-col justify-center gap-6 p-6">
        <div className="text-center">
          <h1 className="text-2xl font-semibold text-[var(--v2-text-strong)]">
            ${t("onboarding.title")}
          </h1>
          <p className="mt-2 text-sm text-[var(--v2-text-muted)]">${t("onboarding.subtitle")}</p>
        </div>

        <div className="flex flex-col gap-3">
          ${featured.map(
            ({ entry, provider }) => html`
              <${FeaturedProviderRow}
                key=${entry.id}
                entry=${entry}
                provider=${provider}
                configured=${isProviderConfigured(provider, state.builtinOverrides)}
                isBusy=${state.isBusy}
                login=${login}
                t=${t}
                onUse=${handleUse}
                onSetUp=${actions.openDialog}
              />
            `
          )}
        </div>

        <${ProviderLoginStatus} login=${login} />

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
