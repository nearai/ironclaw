import React from "react";
import { Button } from "../../../design-system/button";
import { PairingWebCodePanel } from "../../../components/pairing-web-code-panel";
import { useT } from "../../../lib/i18n";
import { channelConnectionDisplayName } from "../../../lib/channel-connection-events";

export function OnboardingPairingCard({ onboarding, onConfigure, onCancel }) {
  const t = useT();
  const [error, setError] = React.useState("");
  const [isConfiguring, setIsConfiguring] = React.useState(false);
  // Derived-from-props during render (not an effect) so the minimal test
  // harness stays useState-only: when the onboarding hook stamps a failed or
  // timed-out OAuth flow onto the card, exit the connect spinner and surface
  // the retryable error — the popup is gone, so this is the only signal.
  const [lastOauthError, setLastOauthError] = React.useState(null);
  const oauthError = onboarding?.oauthError || null;
  if (oauthError !== lastOauthError) {
    setLastOauthError(oauthError);
    if (oauthError) {
      setError(oauthError);
      setIsConfiguring(false);
    }
  }
  const copy = pairingCardCopy(onboarding, t);

  const configure = async () => {
    if (!onConfigure || isConfiguring) return;
    setError("");
    setIsConfiguring(true);
    try {
      await onConfigure(onboarding);
    } catch {
      setError(copy.errorMessage);
      setIsConfiguring(false);
    }
  };

  // Web-minted code strategy: this side generates the code, so render the
  // pairing panel (code + deep link + QR + live connect detection) instead of
  // an input asking the user to paste a code that doesn't exist yet. The
  // panel is vendor-blind: it drives the generic per-extension pairing
  // endpoints and takes its copy from the backend connection requirement.
  if (onboarding?.strategy === "web_generated_code" && onboarding?.extensionName) {
    const instructions = onboarding?.instructions || onboarding?.message || "";
    return (
      <div
        data-testid="onboarding-pairing-card"
        className="mx-auto mt-4 w-full max-w-lg rounded-lg border border-signal/25 bg-signal/5 p-4"
      >
        <h3 className="text-sm font-semibold text-iron-100">{copy.title}</h3>
        {instructions &&
        (<p className="mt-1 text-sm leading-6 text-iron-300">{instructions}</p>)}
        <PairingWebCodePanel
          compact
          extensionId={onboarding.extensionName}
          displayName={copy.displayName || onboarding.extensionName}
        />
        {onCancel &&
        (
          <div className="mt-3">
            <Button
              variant="ghost"
              className="h-9 px-3 text-xs"
              onClick={onCancel}
            >
              {t("common.dismiss")}
            </Button>
          </div>
        )}
      </div>
    );
  }

  // OAuth and administrator-managed strategies have no code that can safely be
  // pasted into chat. Render manifest-authored guidance and the generic
  // configure action when one is available.
  return (
    <div
      data-testid="onboarding-pairing-card"
      className="mx-auto mt-4 w-full max-w-lg rounded-lg border border-signal/25 bg-signal/5 p-4"
    >
      <h3 className="text-sm font-semibold text-iron-100">{copy.title}</h3>
      <p className="mt-1 text-sm leading-6 text-iron-300">{copy.instructions}</p>
      <div className="mt-3 flex flex-wrap gap-2">
        {onConfigure &&
        (
          <Button
            variant="secondary"
            className="h-9 gap-2 px-3 text-xs"
            onClick={configure}
            loading={isConfiguring}
          >
            {isConfiguring ? copy.submittingLabel : copy.submitLabel}
          </Button>
        )}
        {onCancel &&
        (
          <Button
            variant="ghost"
            className="h-9 px-3 text-xs"
            onClick={onCancel}
          >
            {t("common.dismiss")}
          </Button>
        )}
      </div>
      {!onConfigure &&
      (
        <p className="mt-2 text-xs leading-5 text-iron-400">
          {t("pairing.connectFromExtensions", { name: copy.displayName })}
        </p>
      )}
      {error &&
      (<p role="alert" className="mt-3 text-xs leading-5 text-red-300">{error}</p>)}
    </div>
  );
}

function pairingCardCopy(onboarding, t) {
  const displayName = channelConnectionDisplayName(onboarding?.extensionName);
  return {
    displayName,
    title: t("pairing.connectTitle", { name: displayName }),
    instructions:
      onboarding?.instructions ||
      onboarding?.message ||
      t("pairing.connectInstructions", { name: displayName }),
    submitLabel: onboarding?.submitLabel || t("pairing.connect"),
    submittingLabel: onboarding?.submittingLabel || t("connection.connecting"),
    errorMessage: onboarding?.errorMessage || t("pairing.connectFailedRetry"),
  };
}
