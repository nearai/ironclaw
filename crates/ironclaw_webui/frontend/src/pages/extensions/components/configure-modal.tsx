// @ts-nocheck
import { useQueryClient } from "@tanstack/react-query";
import { Button, Icon, Input, Text } from "@ironclaw/design-system";
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
import { resolveFocusTarget } from "../lib/focus-target";
import type { FocusTarget } from "../lib/focus-target";
import { PairingWebCodePanel } from "../../../components/pairing-web-code-panel";

/**
 * @param {{
 *   extension: any;
 *   onClose: () => void;
 *   onSaved?: (result?: unknown) => void;
 *   returnFocusTo?: FocusTarget | null;
 * }} props
 */
export function ConfigureModal({ extension, onClose, onSaved, returnFocusTo }) {
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
        returnFocusTo={returnFocusTo}
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
      <ModalShell
        onClose={onClose}
        returnFocusTo={returnFocusTo}
        title={t("extensions.configureName").replace("{name}", extensionName)}
      >
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
      <ModalShell
        onClose={onClose}
        returnFocusTo={returnFocusTo}
        title={t("extensions.configureName").replace("{name}", extensionName)}
      >
        <Text variant="body" tone="danger">
          {t("extensions.loadFailed")} {error.message}
        </Text>
      </ModalShell>
    );
  }

  if (secrets.length === 0) {
    return (
      <ModalShell
        onClose={onClose}
        returnFocusTo={returnFocusTo}
        title={t("extensions.configureName").replace("{name}", extensionName)}
      >
        <Text variant="body" tone="muted">
          {t("extensions.noConfigRequired")}
        </Text>
      </ModalShell>
    );
  }

  return (
    <ModalShell
      onClose={onClose}
      returnFocusTo={returnFocusTo}
      title={t("extensions.configureName").replace("{name}", extensionName)}
    >
      {onboarding?.credential_instructions &&
      (
        <Text variant="body" tone="muted" className="mb-4">
          {onboarding.credential_instructions}
        </Text>
      )}
      {setupUrl &&
      (
        <a
          href={setupUrl}
          target="_blank"
          rel="noopener noreferrer"
          className="mb-4 inline-flex items-center gap-1.5 text-sm text-[var(--v2-accent-text)] hover:underline"
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
                className="mb-1.5 flex items-center gap-2 text-sm text-[var(--v2-text)]"
              >
                {secret.prompt || secret.name}
                {secret.optional &&
                (
                  <span className="font-mono text-[10px] text-[var(--v2-text-faint)]"
                    >{t("common.optional") || "optional"}</span
                  >
                )}
                {secret.provided &&
                (
                  <span className="font-mono text-[10px] text-[var(--v2-positive-text)]"
                    >{t("common.configured") || "configured"}</span
                  >
                )}
              </label>
              {(secret.setup?.kind || "manual_token") === "oauth"
                ? (
                    <div className="flex items-center justify-between gap-3 rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] px-3 py-2">
                      <Text variant="caption" tone="muted">
                        {secret.provided
                          ? t("extensions.authConfigured")
                          : t("extensions.authPopup")}
                      </Text>
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
              <Input
                size="lg"
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
              />
              {secret.auto_generate &&
              !secret.provided &&
              (
                <Text as="p" variant="caption" tone="faint" className="mt-1">
                  {t("extensions.autoGenerated")}
                </Text>
              )}
              </>
                  )}
            </div>
          )
        )}
      </div>

      {onboarding?.credential_next_step &&
      (
        <Text as="p" variant="caption" tone="muted" className="mt-4">
          {onboarding.credential_next_step}
        </Text>
      )}
      {isActive &&
      (
        <div
          className="mt-4 rounded-md border border-[color-mix(in_srgb,var(--v2-positive-text)_20%,transparent)] bg-[var(--v2-positive-soft)] px-3 py-2 text-xs text-[var(--v2-positive-text)]"
        >
          {t("extensions.activeConfigured")}
        </div>
      )}
      {submitMutation.error &&
      (
        <div
          className="mt-4 rounded-md border border-[color-mix(in_srgb,var(--v2-danger-text)_25%,transparent)] bg-[var(--v2-danger-soft)] px-3 py-2 text-xs text-[var(--v2-danger-text)]"
        >
          {submitMutation.error.message}
        </div>
      )}
      {oauthMutation.error &&
      (
        <div
          className="mt-4 rounded-md border border-[color-mix(in_srgb,var(--v2-danger-text)_25%,transparent)] bg-[var(--v2-danger-soft)] px-3 py-2 text-xs text-[var(--v2-danger-text)]"
        >
          {oauthMutation.error.message}
        </div>
      )}
      {!oauthMutation.error &&
      oauthMutation.authError &&
      (
        <div
          className="mt-4 rounded-md border border-[color-mix(in_srgb,var(--v2-danger-text)_25%,transparent)] bg-[var(--v2-danger-soft)] px-3 py-2 text-xs text-[var(--v2-danger-text)]"
        >
          {oauthMutation.authError}
        </div>
      )}
      {!oauthMutation.error &&
      !oauthMutation.authError &&
      popupBlockedError &&
      (
        <div
          className="mt-4 rounded-md border border-[color-mix(in_srgb,var(--v2-danger-text)_25%,transparent)] bg-[var(--v2-danger-soft)] px-3 py-2 text-xs text-[var(--v2-danger-text)]"
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

const FOCUSABLE_SELECTOR = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled]):not([type='hidden'])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "[contenteditable='true']",
  "[tabindex]:not([tabindex^='-'])",
].join(",");

function isVisible(element) {
  if (typeof element.checkVisibility === "function") {
    return element.checkVisibility({
      checkOpacity: true,
      checkVisibilityCSS: true,
    });
  }

  const style = window.getComputedStyle(element);
  return (
    element.getClientRects().length > 0 &&
    style.display !== "none" &&
    style.visibility !== "hidden" &&
    style.opacity !== "0"
  );
}

function focusableElements(container) {
  if (!container) return [];
  return Array.from(container.querySelectorAll(FOCUSABLE_SELECTOR)).filter(
    (element) =>
      element.tabIndex >= 0 &&
      !element.hidden &&
      element.getAttribute("aria-hidden") !== "true" &&
      isVisible(element),
  );
}

/**
 * @param {{
 *   onClose: () => void;
 *   returnFocusTo?: FocusTarget | null;
 *   title: string;
 *   children: React.ReactNode;
 * }} props
 */
function ModalShell({ onClose, returnFocusTo, title, children }) {
  const t = useT();
  const titleId = React.useId();
  const dialogRef = React.useRef(null);
  React.useEffect(() => {
    const returnTarget = returnFocusTo || document.activeElement;
    const dialog = dialogRef.current;
    const initialFocus = focusableElements(dialog)[0] || dialog;
    initialFocus?.focus({ preventScroll: true });

    const handleKey = (e) => {
      if (e.key === "Escape") {
        onClose();
        return;
      }
      if (e.key !== "Tab") return;

      const focusable = focusableElements(dialog);
      if (focusable.length === 0) {
        e.preventDefault();
        dialog?.focus({ preventScroll: true });
        return;
      }

      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const activeElement = document.activeElement;
      const focusIsOutside = !dialog?.contains(activeElement);
      if (e.shiftKey && (activeElement === first || focusIsOutside)) {
        e.preventDefault();
        last.focus({ preventScroll: true });
      } else if (!e.shiftKey && (activeElement === last || focusIsOutside)) {
        e.preventDefault();
        first.focus({ preventScroll: true });
      }
    };
    window.addEventListener("keydown", handleKey);
    return () => {
      window.removeEventListener("keydown", handleKey);
      const previouslyFocused = resolveFocusTarget(returnTarget);
      if (
        previouslyFocused?.isConnected &&
        typeof previouslyFocused.focus === "function"
      ) {
        previouslyFocused.focus({ preventScroll: true });
      }
    };
  }, [onClose, returnFocusTo]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-[var(--v2-scrim)] backdrop-blur-sm"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        className="v2-panel mx-4 w-full max-w-lg rounded-2xl p-6"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-5 flex items-center justify-between">
          <h3 id={titleId} className="text-lg font-medium text-[var(--v2-text-strong)]">{title}</h3>
          <button
            onClick={onClose}
            aria-label={t("common.close")}
            className="grid h-8 w-8 place-items-center rounded-md text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]"
          >
            <Icon name="close" className="h-4 w-4" />
          </button>
        </div>
        {children}
      </div>
    </div>
  );
}
