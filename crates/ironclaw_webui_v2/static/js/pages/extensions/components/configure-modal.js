import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import {
  useExtensionSetup,
  useOauthSetup,
  useSetupSubmit,
} from "../hooks/useExtensions.js";
import {
  extensionIsActive,
  setupReadyForActivation,
} from "../lib/extension-actions.js";
import { isChannelExtensionKind } from "../lib/extensions-schema.js";
import { redeemPairingCode } from "../lib/pairing-api.js";
import { activateExtension } from "../lib/extensions-api.js";

export function ConfigureModal({ extension, onActivate, onClose, onSaved }) {
  const t = useT();
  const extensionName = extension?.displayName || extension?.packageRef?.id || t("extensions.defaultName");
  const { secrets = [], fields = [], onboarding, isLoading, error } =
    useExtensionSetup(extension?.packageRef);
  const [values, setValues] = React.useState({});
  const [fieldValues, setFieldValues] = React.useState({});
  const oauthMutation = useOauthSetup(extension?.packageRef);

  const submitMutation = useSetupSubmit(extension?.packageRef, (res) => {
    if (res.success !== false) {
      if (onSaved) onSaved(res);
      onClose();
    }
  });

  const handleSubmit = React.useCallback(() => {
    const secretPayload = {};
    for (const [key, val] of Object.entries(values)) {
      const trimmed = (val || "").trim();
      if (trimmed) secretPayload[key] = trimmed;
    }
    submitMutation.mutate({ secrets: secretPayload, fields: fieldValues });
  }, [values, fieldValues, submitMutation]);
  const handleOauth = React.useCallback(
    (secret) => {
      const popup = window.open("about:blank", "_blank", "width=600,height=600");
      if (popup) popup.opener = null;
      oauthMutation.mutate({ secret, popup });
    },
    [oauthMutation]
  );

  // Channel extensions configure their per-user connection (e.g. Slack account
  // pairing) here instead of credential/OAuth fields: redeem a proof code, then
  // best-effort activate so the channel goes live.
  const queryClient = useQueryClient();
  const packageId =
    typeof extension?.packageRef === "string"
      ? extension.packageRef
      : extension?.packageRef?.id || "";
  const channelId = extension?.channel || packageId;
  const isSlackChannel = channelId.toLowerCase() === "slack";
  // Connectable channels (Slack, Telegram, …) are configured by pairing a user
  // account here — never by operator credential/OAuth fields, and never "no
  // configuration required". A freshly-installed channel is in `setup_required`
  // but still needs the user to connect, so render the Connect panel for any
  // channel kind: a connect step before pairing, and a re-pair affordance once
  // connected. Only genuinely-no-config non-channel extensions may fall through
  // to the "no configuration required" branch below.
  const isChannelExtension = isChannelExtensionKind(extension?.kind);
  const isConnectedChannel = isChannelExtension && Boolean(extension?.authenticated);
  const isPairingChannel = isChannelExtension;
  const channelPairingInstructions = isSlackChannel
    ? t("pairing.slackInstructions")
    : t("pairing.instructions");
  const channelPairingPlaceholder = isSlackChannel
    ? t("pairing.slackPlaceholder")
    : t("pairing.placeholder");
  const channelPairingError = isSlackChannel
    ? t("pairing.slackError")
    : t("pairing.error");
  const [pairingCode, setPairingCode] = React.useState("");
  const pairingMutation = useMutation({
    mutationFn: async (code) => {
      const result = await redeemPairingCode(channelId, code);
      try {
        await activateExtension({ id: packageId || channelId });
      } catch {
        console.error("channel activation after pairing failed.");
      }
      return result;
    },
    onSuccess: () => {
      for (const queryKey of [
        ["extensions"],
        ["connectable-channels"],
        ["pairing", channelId],
      ]) {
        queryClient.invalidateQueries({ queryKey });
      }
      if (onSaved) onSaved();
      onClose();
    },
  });
  const submitPairing = React.useCallback(() => {
    const code = pairingCode.trim();
    if (!code || pairingMutation.isPending) return;
    pairingMutation.mutate(code);
  }, [pairingCode, pairingMutation]);

  const manualSecrets = secrets.filter(
    (secret) => (secret.setup?.kind || "manual_token") === "manual_token"
  );
  const canSave = manualSecrets.length > 0 || fields.length > 0;
  const isActive = extensionIsActive(extension);
  const canActivate = setupReadyForActivation({ extension, secrets, fields });
  const setupUrl = httpsUrl(onboarding?.setup_url);

  if (isPairingChannel) {
    return html`
      <${ModalShell}
        onClose=${onClose}
        title=${t("extensions.configureName").replace("{name}", extensionName)}
      >
        ${isConnectedChannel &&
        html`<p className="mb-2 text-xs leading-5 text-mint">
          ${t("pairing.reconnectHint")}
        </p>`}
        <p className="mb-4 text-sm leading-6 text-iron-300">
          ${channelPairingInstructions}
        </p>
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
          <input
            type="text"
            value=${pairingCode}
            onChange=${(event) => setPairingCode(event.target.value)}
            onKeyDown=${(event) => event.key === "Enter" && submitPairing()}
            placeholder=${channelPairingPlaceholder}
            aria-label=${channelPairingPlaceholder}
            className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
          />
          <${Button}
            variant="primary"
            onClick=${submitPairing}
            disabled=${pairingMutation.isPending || !pairingCode.trim()}
          >
            ${pairingMutation.isPending ? t("common.saving") : t("pairing.connect")}
          <//>
        </div>
        ${pairingMutation.isError &&
        html`<p role="alert" className="mt-3 text-xs leading-5 text-red-300">
          ${channelPairingError}
        </p>`}
      <//>
    `;
  }

  if (isLoading) {
    return html`
      <${ModalShell} onClose=${onClose} title=${t("extensions.configureName").replace("{name}", extensionName)}>
        <div className="space-y-3">
          ${[1, 2].map(
            (i) =>
              html`<div
                key=${i}
                className="v2-skeleton h-10 w-full rounded-md"
              />`
          )}
        </div>
      <//>
    `;
  }

  if (error) {
    return html`
      <${ModalShell} onClose=${onClose} title=${t("extensions.configureName").replace("{name}", extensionName)}>
        <p className="text-sm text-red-200">
          ${t("extensions.loadFailed")} ${error.message}
        </p>
      <//>
    `;
  }

  if (secrets.length === 0 && fields.length === 0) {
    return html`
      <${ModalShell} onClose=${onClose} title=${t("extensions.configureName").replace("{name}", extensionName)}>
        <p className="text-sm text-iron-300">
          ${t("extensions.noConfigRequired")}
        </p>
      <//>
    `;
  }

  return html`
    <${ModalShell} onClose=${onClose} title=${t("extensions.configureName").replace("{name}", extensionName)}>
      ${onboarding?.credential_instructions &&
      html`
        <p className="mb-4 text-sm leading-6 text-iron-300">
          ${onboarding.credential_instructions}
        </p>
      `}
      ${setupUrl &&
      html`
        <a
          href=${setupUrl}
          target="_blank"
          rel="noopener noreferrer"
          className="mb-4 inline-flex items-center gap-1.5 text-sm text-signal hover:underline"
        >
          ${t("extensions.getCredentials")}
          <${Icon} name="bolt" className="h-3.5 w-3.5" />
        </a>
      `}

      <div className="space-y-4">
        ${secrets.map(
          (secret) => html`
            <div key=${secret.name}>
              <label
                className="mb-1.5 flex items-center gap-2 text-sm text-iron-200"
              >
                ${secret.prompt || secret.name}
                ${secret.optional &&
                html`
                  <span className="font-mono text-[10px] text-iron-700"
                    >${t("common.optional") || "optional"}</span
                  >
                `}
                ${secret.provided &&
                html`
                  <span className="font-mono text-[10px] text-mint"
                    >${t("common.configured") || "configured"}</span
                  >
                `}
              </label>
              ${(secret.setup?.kind || "manual_token") === "oauth"
                ? html`
                    <div className="flex items-center justify-between gap-3 rounded-md border border-white/12 bg-white/[0.04] px-3 py-2">
                      <span className="text-xs text-iron-300">
                        ${secret.provided
                          ? t("extensions.authConfigured")
                          : t("extensions.authPopup")}
                      </span>
                      <${Button}
                        variant=${secret.provided ? "secondary" : "primary"}
                        onClick=${() => handleOauth(secret)}
                        disabled=${oauthMutation.isPending}
                      >
                        ${oauthMutation.isPending
                          ? t("extensions.opening")
                          : secret.provided
                            ? t("extensions.reconnect")
                            : t("extensions.authorize")}
                      <//>
                    </div>
                  `
                : html`
              <input
                type="password"
                placeholder=${secret.provided
                  ? t("extensions.keepSecretPlaceholder")
                  : ""}
                value=${values[secret.name] || ""}
                onChange=${(e) =>
                  setValues((prev) => ({
                    ...prev,
                    [secret.name]: e.target.value,
                  }))}
                onKeyDown=${(e) => e.key === "Enter" && handleSubmit()}
                className="h-10 w-full rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
              />
              ${secret.auto_generate &&
              !secret.provided &&
              html`
                <p className="mt-1 text-xs text-iron-700">
                  ${t("extensions.autoGenerated")}
                </p>
              `}
                  `}
            </div>
          `
        )}
        ${fields.map(
          (field) => html`
            <div key=${field.name}>
              <label
                className="mb-1.5 flex items-center gap-2 text-sm text-iron-200"
              >
                ${field.prompt || field.name}
                ${field.optional &&
                html`
                  <span className="font-mono text-[10px] text-iron-700"
                    >${t("common.optional") || "optional"}</span
                  >
                `}
              </label>
              <input
                type="text"
                placeholder=${field.placeholder || ""}
                value=${fieldValues[field.name] || ""}
                onChange=${(e) =>
                  setFieldValues((prev) => ({
                    ...prev,
                    [field.name]: e.target.value,
                  }))}
                onKeyDown=${(e) => e.key === "Enter" && handleSubmit()}
                className="h-10 w-full rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
              />
            </div>
          `
        )}
      </div>

      ${onboarding?.credential_next_step &&
      html`
        <p className="mt-4 text-xs leading-5 text-iron-300">
          ${onboarding.credential_next_step}
        </p>
      `}
      ${isActive &&
      html`
        <div
          className="mt-4 rounded-md border border-mint/20 bg-mint/10 px-3 py-2 text-xs text-mint"
        >
          ${t("extensions.activeConfigured")}
        </div>
      `}
      ${submitMutation.error &&
      html`
        <div
          className="mt-4 rounded-md border border-red-400/20 bg-red-500/10 px-3 py-2 text-xs text-red-200"
        >
          ${submitMutation.error.message}
        </div>
      `}
      ${oauthMutation.error &&
      html`
        <div
          className="mt-4 rounded-md border border-red-400/20 bg-red-500/10 px-3 py-2 text-xs text-red-200"
        >
          ${oauthMutation.error.message}
        </div>
      `}

      <div className="mt-6 flex items-center justify-end gap-3">
        <${Button} variant="ghost" onClick=${onClose}>${t("common.cancel")}<//>
        ${canActivate &&
        html`
        <${Button}
          variant="primary"
          onClick=${() => onActivate?.(extension)}
        >
          ${t("extensions.activate")}
        <//>
        `}
        ${canSave &&
        html`
        <${Button}
          variant=${canActivate ? "secondary" : "primary"}
          onClick=${handleSubmit}
          disabled=${submitMutation.isPending}
        >
          ${submitMutation.isPending ? t("common.saving") : t("common.save")}
        <//>
        `}
      </div>
    <//>
  `;
}

function httpsUrl(value) {
  if (!value) return null;
  try {
    const url = new URL(String(value));
    return url.protocol === "https:" ? url.href : null;
  } catch {
    return null;
  }
}

function ModalShell({ onClose, title, children }) {
  const titleId = React.useId();
  React.useEffect(() => {
    const handleKey = (e) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [onClose]);

  return html`
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
      onClick=${(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby=${titleId}
        className="v2-panel mx-4 w-full max-w-lg rounded-2xl p-6"
        onClick=${(e) => e.stopPropagation()}
      >
        <div className="mb-5 flex items-center justify-between">
          <h3 id=${titleId} className="text-lg font-semibold text-white">${title}</h3>
          <button
            onClick=${onClose}
            className="grid h-8 w-8 place-items-center rounded-md text-iron-300 hover:bg-white/[0.06] hover:text-white"
          >
            <${Icon} name="close" className="h-4 w-4" />
          </button>
        </div>
        ${children}
      </div>
    </div>
  `;
}
