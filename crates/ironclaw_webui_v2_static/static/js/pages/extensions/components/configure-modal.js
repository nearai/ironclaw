import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import {
  useExtensionSetup,
  useOauthSetup,
  useSetupSubmit,
} from "../hooks/useExtensions.js";
import { setupReadyForActivation } from "../lib/extension-actions.js";

export function ConfigureModal({ extension, onActivate, onClose, onSaved }) {
  const t = useT();
  const extensionName = extension?.displayName || extension?.packageRef?.id || "Extension";
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
  const manualSecrets = secrets.filter(
    (secret) => (secret.setup?.kind || "manual_token") === "manual_token"
  );
  const canSave = manualSecrets.length > 0 || fields.length > 0;
  const canActivate = setupReadyForActivation({ secrets, fields });

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
          ${t("extensions.loadFailed") || "Failed to load setup:"} ${error.message}
        </p>
      <//>
    `;
  }

  if (secrets.length === 0 && fields.length === 0) {
    return html`
      <${ModalShell} onClose=${onClose} title=${t("extensions.configureName").replace("{name}", extensionName)}>
        <p className="text-sm text-iron-300">
          ${t("extensions.noConfigRequired") || "No configuration required for this extension."}
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
      ${onboarding?.setup_url &&
      html`
        <a
          href=${onboarding.setup_url}
          target="_blank"
          rel="noopener noreferrer"
          className="mb-4 inline-flex items-center gap-1.5 text-sm text-signal hover:underline"
        >
          Get credentials
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
                          ? t("extensions.authConfigured") || "Authorization is configured."
                          : t("extensions.authPopup") || "Authorize this provider in a browser popup."}
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
                  ? "••••••• (leave blank to keep)"
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
                  ${t("extensions.autoGenerated") || "Auto-generated if left blank"}
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
        <${Button} variant="ghost" onClick=${onClose}>${t("common.cancel") || "Cancel"}<//>
        ${canActivate &&
        html`
        <${Button}
          variant="primary"
          onClick=${() => onActivate?.(extension)}
        >
          Activate
        <//>
        `}
        ${canSave &&
        html`
        <${Button}
          variant=${canActivate ? "secondary" : "primary"}
          onClick=${handleSubmit}
          disabled=${submitMutation.isPending}
        >
          ${submitMutation.isPending ? "Saving…" : t("common.save") || "Save"}
        <//>
        `}
      </div>
    <//>
  `;
}

function ModalShell({ onClose, title, children }) {
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
        className="v2-panel mx-4 w-full max-w-lg rounded-2xl p-6"
        onClick=${(e) => e.stopPropagation()}
      >
        <div className="mb-5 flex items-center justify-between">
          <h3 className="text-lg font-semibold text-white">${title}</h3>
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
