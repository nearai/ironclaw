// @ts-nocheck
import { useQueryClient } from "@tanstack/react-query";
import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import React from "react";
import { useT } from "../../../lib/i18n";
import {
  useExtensionSetup,
  useOauthSetup,
  useSetupSubmit,
} from "../hooks/useExtensions";
import {
  extensionIsActive,
} from "../lib/extension-actions";
import {
  channelConnection,
  hasChannelSurface,
  isWebGeneratedCodeConnection,
} from "../lib/extensions-schema";
import { PairingWebCodePanel } from "../../../components/pairing-web-code-panel";

export function ConfigureModal({ extension, onClose, onSaved }) {
  const t = useT();
  const extensionName = extension?.displayName || extension?.packageRef?.id || t("extensions.defaultName");
  const { secrets = [], onboarding, isLoading, error } =
    useExtensionSetup(extension?.packageRef);
  const [values, setValues] = React.useState({});
  const queryClient = useQueryClient();
  const packageId =
    typeof extension?.packageRef === "string"
      ? extension.packageRef
      : extension?.packageRef?.id || "";
  const handleOauthConfigured = React.useCallback(async () => {
    onClose();
    // The server-owned OAuth continuation performs lifecycle activation and
    // connection fan-out transactionally. The browser only refreshes the
    // authoritative caller-scoped projection after callback completion.
    await Promise.all(
      [["extensions"], ["extension-registry"], ["extension-setup", packageId]].map(
        (queryKey) => queryClient.invalidateQueries({ queryKey }),
      ),
    );
    if (onSaved) onSaved();
  }, [onClose, onSaved, packageId, queryClient]);
  const oauthMutation = useOauthSetup(extension?.packageRef, {
    onConfigured: handleOauthConfigured,
  });

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
    submitMutation.mutate({ secrets: secretPayload });
  }, [values, submitMutation]);
  const [popupBlockedError, setPopupBlockedError] = React.useState("");
  const handleOauth = React.useCallback(
    (secret) => {
      const popup = window.open("about:blank", "_blank", "width=600,height=600");
      if (popup) popup.opener = null;
      // Unlike the later noopener open (which returns null even on success
      // per spec), a null pre-open reliably means the browser blocked the
      // popup — surface it and stop before burning the OAuth flow start,
      // mirroring the in-chat startOnboardingOAuth guard.
      if (!popup) {
        setPopupBlockedError(t("authGate.popupBlocked"));
        return;
      }
      setPopupBlockedError("");
      oauthMutation.mutate({ secret, popup });
    },
    [oauthMutation, t]
  );

  const manualSecrets = secrets.filter(
    (secret) => (secret.setup?.kind || "manual_token") === "manual_token"
  );
  // The manifest declares whether the user-facing setup is a host-issued
  // code/deep-link/QR flow. Do not probe a provider route to infer strategy.
  const connection = channelConnection(extension);
  const isWebCodeChannel =
    hasChannelSurface(extension) &&
    isWebGeneratedCodeConnection(connection);

  const canSave = manualSecrets.length > 0;
  const isActive = extensionIsActive(extension);
  const oauthBusy = oauthMutation.isPending || oauthMutation.isAuthorizing;
  const setupUrl = httpsUrl(onboarding?.setup_url);
  if (isWebCodeChannel) {
    // The panel is self-contained (mints/rotates codes, polls status,
    // broadcasts channel-connected on pairing), so the modal only hosts it.
    return (
      <ModalShell
        onClose={onClose}
        title={t("extensions.configureName").replace("{name}", extensionName)}
      >
        <PairingWebCodePanel
          extensionId={packageId}
          displayName={extensionName}
          instructions={connection?.instructions || ""}
          compact
        />
      </ModalShell>
    );
  }

  if (isLoading) {
    return (
      <ModalShell onClose={onClose} title={t("extensions.configureName").replace("{name}", extensionName)}>
        <div className="space-y-3">
          {[1, 2].map(
            (i) =>
              (<div
                key={i}
                className="v2-skeleton h-10 w-full rounded-md"
              />)
          )}
        </div>
      </ModalShell>
    );
  }

  if (error) {
    return (
      <ModalShell onClose={onClose} title={t("extensions.configureName").replace("{name}", extensionName)}>
        <p className="text-sm text-red-200">
          {t("extensions.loadFailed")} {error.message}
        </p>
      </ModalShell>
    );
  }

  if (secrets.length === 0) {
    return (
      <ModalShell onClose={onClose} title={t("extensions.configureName").replace("{name}", extensionName)}>
        <p className="text-sm text-iron-300">
          {t("extensions.noConfigRequired")}
        </p>
      </ModalShell>
    );
  }

  return (
    <ModalShell onClose={onClose} title={t("extensions.configureName").replace("{name}", extensionName)}>
      {onboarding?.credential_instructions &&
      (
        <p className="mb-4 text-sm leading-6 text-iron-300">
          {onboarding.credential_instructions}
        </p>
      )}
      {setupUrl &&
      (
        <a
          href={setupUrl}
          target="_blank"
          rel="noopener noreferrer"
          className="mb-4 inline-flex items-center gap-1.5 text-sm text-signal hover:underline"
        >
          {t("extensions.getCredentials")}
          <Icon name="bolt" className="h-3.5 w-3.5" />
        </a>
      )}

      <div className="space-y-4">
        {secrets.map(
          (secret) => (
            <div key={secret.name}>
              <label
                className="mb-1.5 flex items-center gap-2 text-sm text-iron-200"
              >
                {secret.prompt || secret.name}
                {secret.optional &&
                (
                  <span className="font-mono text-[10px] text-iron-700"
                    >{t("common.optional") || "optional"}</span
                  >
                )}
                {secret.provided &&
                (
                  <span className="font-mono text-[10px] text-mint"
                    >{t("common.configured") || "configured"}</span
                  >
                )}
              </label>
              {(secret.setup?.kind || "manual_token") === "oauth"
                ? (
                    <div className="flex items-center justify-between gap-3 rounded-md border border-white/12 bg-white/[0.04] px-3 py-2">
                      <span className="text-xs text-iron-300">
                        {secret.provided
                          ? t("extensions.authConfigured")
                          : t("extensions.authPopup")}
                      </span>
                      <Button
                        variant={secret.provided ? "secondary" : "primary"}
                        onClick={() => handleOauth(secret)}
                        loading={oauthBusy}
                      >
                        {oauthBusy
                          ? t("extensions.opening")
                          : secret.provided
                            ? t("extensions.reconnect")
                            : t("extensions.authorize")}
                      </Button>
                    </div>
                  )
                : (
              <>
              <input
                type="password"
                placeholder={secret.provided
                  ? t("extensions.keepSecretPlaceholder")
                  : ""}
                value={values[secret.name] || ""}
                onChange={(e) => {
                  const value = e.currentTarget.value;
                  setValues((prev) => ({
                    ...prev,
                    [secret.name]: value,
                  }));
                }}
                onKeyDown={(e) => e.key === "Enter" && handleSubmit()}
                className="h-10 w-full rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
              />
              {secret.auto_generate &&
              !secret.provided &&
              (
                <p className="mt-1 text-xs text-iron-700">
                  {t("extensions.autoGenerated")}
                </p>
              )}
              </>
                  )}
            </div>
          )
        )}
      </div>

      {onboarding?.credential_next_step &&
      (
        <p className="mt-4 text-xs leading-5 text-iron-300">
          {onboarding.credential_next_step}
        </p>
      )}
      {isActive &&
      (
        <div
          className="mt-4 rounded-md border border-mint/20 bg-mint/10 px-3 py-2 text-xs text-mint"
        >
          {t("extensions.activeConfigured")}
        </div>
      )}
      {submitMutation.error &&
      (
        <div
          className="mt-4 rounded-md border border-red-400/20 bg-red-500/10 px-3 py-2 text-xs text-red-200"
        >
          {submitMutation.error.message}
        </div>
      )}
      {oauthMutation.error &&
      (
        <div
          className="mt-4 rounded-md border border-red-400/20 bg-red-500/10 px-3 py-2 text-xs text-red-200"
        >
          {oauthMutation.error.message}
        </div>
      )}
      {!oauthMutation.error &&
      oauthMutation.authError &&
      (
        <div
          className="mt-4 rounded-md border border-red-400/20 bg-red-500/10 px-3 py-2 text-xs text-red-200"
        >
          {oauthMutation.authError}
        </div>
      )}
      {!oauthMutation.error &&
      !oauthMutation.authError &&
      popupBlockedError &&
      (
        <div
          className="mt-4 rounded-md border border-red-400/20 bg-red-500/10 px-3 py-2 text-xs text-red-200"
        >
          {popupBlockedError}
        </div>
      )}

      <div className="mt-6 flex items-center justify-end gap-3">
        <Button variant="ghost" onClick={onClose}>{t("common.cancel")}</Button>
        {canSave &&
        (
        <Button
          variant="primary"
          onClick={handleSubmit}
          loading={submitMutation.isPending}
        >
          {submitMutation.isPending ? t("common.saving") : t("common.save")}
        </Button>
        )}
      </div>
    </ModalShell>
  );
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
  const t = useT();
  const titleId = React.useId();
  React.useEffect(() => {
    const handleKey = (e) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [onClose]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="v2-panel mx-4 w-full max-w-lg rounded-2xl p-6"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-5 flex items-center justify-between">
          <h3 id={titleId} className="text-lg font-semibold text-white">{title}</h3>
          <button
            onClick={onClose}
            aria-label={t("common.close")}
            className="grid h-8 w-8 place-items-center rounded-md text-iron-300 hover:bg-white/[0.06] hover:text-white"
          >
            <Icon name="close" className="h-4 w-4" />
          </button>
        </div>
        {children}
      </div>
    </div>
  );
}
