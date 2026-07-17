import React from "react";
import { Button } from "../../../design-system/button";
import { useT } from "../../../lib/i18n";
import { channelConnectionDisplayName } from "../../../lib/channel-connection-events";

// Strategies whose in-chat affordance is "paste a code". OAuth gets a direct
// configure button; QR/admin-managed channels render guidance instead of a code
// box they can't use. An absent strategy defaults to the paste panel for
// backward compatibility.
const PASTE_CODE_STRATEGIES = new Set(["inbound_proof_code", "web_generated_code"]);

function acceptsPastedCode(strategy) {
  return !strategy || PASTE_CODE_STRATEGIES.has(strategy);
}

export function OnboardingPairingCard({ onboarding, onSubmit, onConfigure, onCancel }) {
  const t = useT();
  const [code, setCode] = React.useState("");
  const [error, setError] = React.useState("");
  // "idle" -> "submitting" (redeem in flight) -> "resuming" (redeem succeeded;
  // hold the spinner while the parked turn resumes and this gate clears).
  const [status, setStatus] = React.useState("idle");
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
  const busy = status !== "idle";

  const submit = async () => {
    const trimmed = code.trim();
    if (!trimmed || busy) return;
    setError("");
    setStatus("submitting");
    try {
      await onSubmit(trimmed);
      setCode("");
      // Success: don't snap back to idle. The redeem resolved, but the backend
      // resumes the parked turn asynchronously and the projection clears this
      // gate (unmounting the card) a beat later over SSE. Holding the spinner
      // keeps a successful submit from looking like it did nothing.
      setStatus("resuming");
    } catch (submitError) {
      // A resume fault means the connection succeeded but this parked chat
      // didn't continue; the gate won't clear, so exit the spinner with copy
      // that says so rather than the generic invalid-code message.
      setError(submitError?.resumeFailed ? copy.resumeFailedMessage : copy.errorMessage);
      setStatus("idle");
    }
  };

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

  // Non-paste strategy: render the channel's configured connection action rather
  // than a code box that would submit a meaningless value.
  if (!acceptsPastedCode(onboarding?.strategy)) {
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

  return (
    <div
      data-testid="onboarding-pairing-card"
      className="mx-auto mt-4 w-full max-w-lg rounded-lg border border-signal/25 bg-signal/5 p-4"
    >
      <div className="mb-3">
        <h3 className="text-sm font-semibold text-iron-100">{copy.title}</h3>
        <p className="mt-1 text-sm leading-6 text-iron-300">{copy.instructions}</p>
      </div>

      <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
        <input
          type="text"
          value={code}
          onChange={(event) => setCode(event.currentTarget.value)}
          onKeyDown={(event) => event.key === "Enter" && submit()}
          placeholder={copy.placeholder}
          aria-label={copy.placeholder}
          disabled={busy}
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45 disabled:cursor-not-allowed disabled:opacity-60"
        />
        <Button
          variant="secondary"
          className="h-9 shrink-0 gap-2 px-3 text-xs"
          onClick={submit}
          loading={busy}
          disabled={!code.trim()}
        >
          {busy ? copy.submittingLabel : copy.submitLabel}
        </Button>
        {onCancel &&
        (
          <Button
            variant="ghost"
            className="h-9 shrink-0 px-3 text-xs"
            onClick={onCancel}
            disabled={busy}
          >
            {t("common.cancel")}
          </Button>
        )}
      </div>

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
      t("pairing.openAndPaste", { name: displayName }),
    placeholder: onboarding?.inputPlaceholder || t("pairing.placeholder"),
    submitLabel: onboarding?.submitLabel || t("pairing.connect"),
    submittingLabel: onboarding?.submittingLabel || t("connection.connecting"),
    errorMessage: onboarding?.errorMessage || t("pairing.checkCodeAndRetry"),
    resumeFailedMessage:
      onboarding?.resumeFailedMessage ||
      t("pairing.resumeFailed", { name: displayName }),
  };
}
