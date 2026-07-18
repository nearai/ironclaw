// @ts-nocheck
import { useMutation, useQueryClient } from "@tanstack/react-query";
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
  extensionLifecycleState,
  setupReadyForActivation,
} from "../lib/extension-actions";
import { isChannelExtensionKind } from "../lib/extensions-schema";
import { redeemPairingCode } from "../lib/pairing-api";
import { activateExtension } from "../lib/extensions-api";
import { notifyChannelConnected } from "../../../lib/channel-connection-events";

// Model B: the visible Slack extension is the user-tools package (id `slack`),
// not the bot channel (which is hidden operator infrastructure).
const SLACK_TOOLS_EXTENSION_ID = "slack";

export function ConfigureModal({ extension, onActivate, onClose, onSaved }) {
  const t = useT();
  const extensionName = extension?.displayName || extension?.packageRef?.id || t("extensions.defaultName");
  const { secrets = [], fields = [], onboarding, isLoading, error } =
    useExtensionSetup(extension?.packageRef);
  const [values, setValues] = React.useState({});
  const [fieldValues, setFieldValues] = React.useState({});
  const queryClient = useQueryClient();
  const packageId =
    typeof extension?.packageRef === "string"
      ? extension.packageRef
      : extension?.packageRef?.id || "";
  const channelId = extension?.channel || packageId;
  const lifecycleState = extensionLifecycleState(extension);
  // Slack tools use OAuth rather than the proof-code pairing flow below.
  const isSlackToolsExtension =
    channelId.toLowerCase() === SLACK_TOOLS_EXTENSION_ID;
  const handleOauthConfigured = React.useCallback(async () => {
    // Extension-scoped OAuth completion is atomic on the backend: the callback
    // is not marked complete until lifecycle activation has published tools.
    // A second client-side activation races that committed state and used to
    // surface a misleading Conflict after an otherwise successful popup.
    // invalidateQueries refetches active queries and resolves when they
    // settle (TanStack v5), so no follow-up refetchQueries pass is needed.
    await Promise.all(
      [["extensions"], ["extension-registry"], ["extension-setup", packageId]].map(
        (queryKey) => queryClient.invalidateQueries({ queryKey }),
      ),
    );
    // Broadcast channel-connected (same event pairing redemption sends) so an
    // open chat card for this channel clears and its parked request resumes —
    // connecting from the Extensions page must not strand the chat surface.
    if ((isChannelExtensionKind(extension?.kind) || isSlackToolsExtension) && channelId) {
      try {
        await notifyChannelConnected({ channel: channelId, source: "extensions-oauth" });
      } catch {
        console.error("channel connection broadcast after OAuth failed.");
      }
    }
    if (onSaved) onSaved();
    onClose();
  }, [channelId, extension?.kind, isSlackToolsExtension, onClose, onSaved, packageId, queryClient]);
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
    submitMutation.mutate({ secrets: secretPayload, fields: fieldValues });
  }, [values, fieldValues, submitMutation]);
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
        setPopupBlockedError("Authorization popup was blocked.");
        return;
      }
      setPopupBlockedError("");
      oauthMutation.mutate({ secret, popup });
    },
    [oauthMutation]
  );

  // Some channel extensions may still use proof-code setup: redeem a code,
  // then best-effort activate so the channel goes live.
  const oauthSecrets = secrets.filter(
    (secret) => (secret.setup?.kind || "manual_token") === "oauth"
  );
  const manualSecrets = secrets.filter(
    (secret) => (secret.setup?.kind || "manual_token") === "manual_token"
  );
  const isPairingChannel =
    !isSlackToolsExtension &&
    isChannelExtensionKind(extension?.kind) &&
    (lifecycleState === "pairing" || lifecycleState === "pairing_required");
  const channelPairingInstructions = t("pairing.instructions");
  const channelPairingPlaceholder = t("pairing.placeholder");
  const channelPairingError = t("pairing.error");
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

  const canSave = manualSecrets.length > 0 || fields.length > 0;
  const isActive = extensionIsActive(extension);
  const canActivate =
    !isChannelExtensionKind(extension?.kind) &&
    setupReadyForActivation({ extension, secrets, fields });
  const oauthBusy = oauthMutation.isPending || oauthMutation.isAuthorizing;
  const setupUrl = httpsUrl(onboarding?.setup_url);

  if (isPairingChannel) {
    return (
      <ModalShell
        onClose={onClose}
        title={t("extensions.configureName").replace("{name}", extensionName)}
      >
        <p className="mb-4 text-sm leading-6 text-iron-300">
          {channelPairingInstructions}
        </p>
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
          <input
            type="text"
            value={pairingCode}
            onChange={(event) => setPairingCode(event.currentTarget.value)}
            onKeyDown={(event) => event.key === "Enter" && submitPairing()}
            placeholder={channelPairingPlaceholder}
            aria-label={channelPairingPlaceholder}
            className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
          />
          <Button
            variant="primary"
            onClick={submitPairing}
            loading={pairingMutation.isPending}
            disabled={!pairingCode.trim()}
          >
            {pairingMutation.isPending ? t("common.saving") : t("pairing.connect")}
          </Button>
        </div>
        {pairingMutation.isError &&
        (<p role="alert" className="mt-3 text-xs leading-5 text-red-300">
          {channelPairingError}
        </p>)}
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

  if (secrets.length === 0 && fields.length === 0) {
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
        {fields.map(
          (field) => (
            <div key={field.name}>
              <label
                className="mb-1.5 flex items-center gap-2 text-sm text-iron-200"
              >
                {field.prompt || field.name}
                {field.optional &&
                (
                  <span className="font-mono text-[10px] text-iron-700"
                    >{t("common.optional") || "optional"}</span
                  >
                )}
              </label>
              <input
                type="text"
                placeholder={field.placeholder || ""}
                value={fieldValues[field.name] || ""}
                onChange={(e) => {
                  const value = e.currentTarget.value;
                  setFieldValues((prev) => ({
                    ...prev,
                    [field.name]: value,
                  }));
                }}
                onKeyDown={(e) => e.key === "Enter" && handleSubmit()}
                className="h-10 w-full rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
              />
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
        {canActivate &&
        (
        <Button
          variant="primary"
          onClick={() => onActivate?.(extension)}
        >
          {t("extensions.activate")}
        </Button>
        )}
        {canSave &&
        (
        <Button
          variant={canActivate ? "secondary" : "primary"}
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
