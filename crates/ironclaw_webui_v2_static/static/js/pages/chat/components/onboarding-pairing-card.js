import { React, html } from "../../../lib/html.js";
import { Button } from "../../../design-system/button.js";
import { channelConnectionDisplayName } from "../../../lib/channel-connection-events.js";

// Strategies whose in-chat affordance is "paste a code". Other strategies (OAuth,
// QR, admin-managed channels) can't be completed from this inline panel, so the
// user is pointed at the Extensions page rather than shown a code box they can't
// use. An absent strategy defaults to the paste panel for backward compatibility.
const PASTE_CODE_STRATEGIES = new Set(["inbound_proof_code", "web_generated_code"]);

function acceptsPastedCode(strategy) {
  return !strategy || PASTE_CODE_STRATEGIES.has(strategy);
}

export function OnboardingPairingCard({ onboarding, onSubmit, onCancel }) {
  const [code, setCode] = React.useState("");
  const [error, setError] = React.useState("");
  const [isSubmitting, setIsSubmitting] = React.useState(false);
  const copy = pairingCardCopy(onboarding);

  const submit = async () => {
    const trimmed = code.trim();
    if (!trimmed || isSubmitting) return;
    setError("");
    setIsSubmitting(true);
    try {
      await onSubmit(trimmed);
      setCode("");
    } catch (err) {
      setError(err?.message || copy.errorMessage);
    } finally {
      setIsSubmitting(false);
    }
  };

  // Non-paste strategy: this channel is connected from the Extensions page, not by
  // pasting a code here. Render guidance instead of a code box that would submit a
  // meaningless value.
  if (!acceptsPastedCode(onboarding?.strategy)) {
    return html`
      <div
        data-testid="onboarding-pairing-card"
        className="mx-auto mt-4 w-full max-w-lg rounded-lg border border-signal/25 bg-signal/5 p-4"
      >
        <h3 className="text-sm font-semibold text-iron-100">${copy.title}</h3>
        <p className="mt-1 text-sm leading-6 text-iron-300">
          Connect ${copy.displayName} from the Extensions page to continue.
        </p>
        ${onCancel &&
        html`
          <div className="mt-3">
            <${Button} variant="ghost" className="h-9 px-3 text-xs" onClick=${onCancel}>
              Dismiss
            <//>
          </div>
        `}
      </div>
    `;
  }

  return html`
    <div
      data-testid="onboarding-pairing-card"
      className="mx-auto mt-4 w-full max-w-lg rounded-lg border border-signal/25 bg-signal/5 p-4"
    >
      <div className="mb-3">
        <h3 className="text-sm font-semibold text-iron-100">${copy.title}</h3>
        <p className="mt-1 text-sm leading-6 text-iron-300">${copy.instructions}</p>
      </div>

      <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
        <input
          type="text"
          value=${code}
          onChange=${(event) => setCode(event.target.value)}
          onKeyDown=${(event) => event.key === "Enter" && submit()}
          placeholder=${copy.placeholder}
          aria-label=${copy.placeholder}
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <${Button}
          variant="secondary"
          className="h-9 shrink-0 px-3 text-xs"
          onClick=${submit}
          disabled=${isSubmitting || !code.trim()}
        >
          ${isSubmitting ? copy.submittingLabel : copy.submitLabel}
        <//>
        ${onCancel &&
        html`
          <${Button}
            variant="ghost"
            className="h-9 shrink-0 px-3 text-xs"
            onClick=${onCancel}
            disabled=${isSubmitting}
          >
            Cancel
          <//>
        `}
      </div>

      ${error &&
      html`<p role="alert" className="mt-3 text-xs leading-5 text-red-300">${error}</p>`}
    </div>
  `;
}

function pairingCardCopy(onboarding) {
  const displayName = channelConnectionDisplayName(onboarding?.extensionName);
  return {
    displayName,
    title: `Connect ${displayName}`,
    instructions:
      onboarding?.instructions ||
      onboarding?.message ||
      `Open ${displayName}, get the pairing code, and paste it here.`,
    placeholder: onboarding?.inputPlaceholder || "Enter pairing code",
    submitLabel: onboarding?.submitLabel || "Connect",
    submittingLabel: onboarding?.submittingLabel || "Connecting...",
    errorMessage: onboarding?.errorMessage || "Pairing failed. Check the code and try again.",
  };
}
