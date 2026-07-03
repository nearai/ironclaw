import { React, html } from "../lib/html.js";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "../design-system/button.js";
import { useT } from "../lib/i18n.js";
import { redeemSlackPairingCode } from "../lib/slack-pairing-api.js";

export function SlackPairingSection({ action }) {
  const t = useT();
  const queryClient = useQueryClient();
  const redeemMutation = useMutation({
    mutationFn: ({ code }) => redeemSlackPairingCode(code),
    onSuccess: () => {
      setManualCode("");
      queryClient.invalidateQueries({ queryKey: ["extensions"] });
      queryClient.invalidateQueries({ queryKey: ["connectable-channels"] });
      queryClient.invalidateQueries({ queryKey: ["pairing", "slack"] });
    },
  });
  const [manualCode, setManualCode] = React.useState("");
  const copy = slackPairingCopy(action, t);

  const submit = () => {
    if (redeemMutation.isPending) return;
    const code = manualCode.trim().toUpperCase();
    if (!code) return;
    redeemMutation.mutate({ code });
  };

  return html`
    <div
      data-testid="slack-pairing-section"
      className="mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4"
    >
      <h4 className="mb-3 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
        ${copy.title}
      </h4>
      <p className="mb-4 text-xs leading-5 text-iron-300">
        ${copy.instructions}
      </p>

      <div className="mb-3 flex flex-col gap-2 sm:flex-row sm:items-center">
        <input
          type="text"
          value=${manualCode}
          onChange=${(event) => setManualCode(event.target.value)}
          onKeyDown=${(event) => event.key === "Enter" && submit()}
          placeholder=${copy.codePlaceholder}
          data-testid="slack-pairing-code-input"
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <${Button}
          variant="secondary"
          className="h-9 shrink-0 px-3 text-xs"
          data-testid="slack-pairing-submit"
          onClick=${submit}
          disabled=${redeemMutation.isPending || !manualCode.trim()}
        >
          ${copy.submitLabel}
        <//>
      </div>

      ${redeemMutation.isSuccess &&
      html`<p data-testid="slack-pairing-success" className="text-xs text-emerald-300">
        ${redeemMutation.data?.message || copy.successMessage}
      </p>`}
      ${redeemMutation.isError &&
      html`<p data-testid="slack-pairing-error" className="text-xs text-red-300">
        ${slackPairingError(redeemMutation.error, copy.errorMessage)}
      </p>`}
    </div>
  `;
}

function slackPairingCopy(action, t) {
  return {
    title: action?.title || t("pairing.slackTitle"),
    instructions: action?.instructions || t("pairing.slackInstructions"),
    codePlaceholder:
      action?.input_placeholder || action?.code_placeholder || t("pairing.slackPlaceholder"),
    submitLabel: action?.submit_label || t("pairing.connect"),
    successMessage: action?.success_message || t("pairing.slackSuccess"),
    errorMessage: action?.error_message || t("pairing.slackError"),
  };
}

function slackPairingError(error, fallback) {
  return error?.payload?.error || error?.payload?.message || error?.message || fallback;
}
