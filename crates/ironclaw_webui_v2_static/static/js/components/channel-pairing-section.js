import { useMutation, useQueryClient } from "@tanstack/react-query";
import { React, html } from "../lib/html.js";
import { apiFetch } from "../lib/api.js";
import { Button } from "../design-system/button.js";
import { useT } from "../lib/i18n.js";

const PAIRING_REDEEM_PATH = "/api/webchat/v2/extensions/pairing/redeem";

export function ChannelPairingSection({ channel, action }) {
  const t = useT();
  const queryClient = useQueryClient();
  const [manualCode, setManualCode] = React.useState("");
  const copy = channelPairingCopy(action, t);
  const redeemMutation = useMutation({
    mutationFn: ({ code }) => redeemChannelPairingCode(channel, code),
    onSuccess: () => {
      setManualCode("");
      queryClient.invalidateQueries({ queryKey: ["extensions"] });
      queryClient.invalidateQueries({ queryKey: ["connectable-channels"] });
      queryClient.invalidateQueries({ queryKey: ["pairing", channel] });
    },
  });

  const submit = () => {
    if (redeemMutation.isPending) return;
    const code = manualCode.trim().toUpperCase();
    if (!code) return;
    redeemMutation.mutate({ code });
  };

  return html`
    <div
      data-testid="pairing-section"
      className="mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4"
    >
      <h4 className="mb-3 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
        ${copy.title}
      </h4>
      <p className="mb-4 text-xs leading-5 text-iron-300">${copy.instructions}</p>

      <div className="mb-3 flex flex-col gap-2 sm:flex-row sm:items-center">
        <input
          type="text"
          value=${manualCode}
          onChange=${(event) => setManualCode(event.target.value)}
          onKeyDown=${(event) => event.key === "Enter" && submit()}
          placeholder=${copy.placeholder}
          data-testid="pairing-code-input"
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <${Button}
          variant="secondary"
          className="h-9 shrink-0 px-3 text-xs"
          onClick=${submit}
          disabled=${redeemMutation.isPending || !manualCode.trim()}
          data-testid="pairing-submit"
        >
          ${copy.submitLabel}
        <//>
      </div>

      ${redeemMutation.isSuccess &&
      html`<p data-testid="pairing-success" className="text-xs text-emerald-300">
        ${redeemMutation.data?.message || copy.successMessage}
      </p>`}
      ${redeemMutation.isError &&
      html`<p data-testid="pairing-error" className="text-xs text-red-300">
        ${channelPairingError(redeemMutation.error, copy.errorMessage)}
      </p>`}
    </div>
  `;
}

function redeemChannelPairingCode(channel, code) {
  return apiFetch(PAIRING_REDEEM_PATH, {
    method: "POST",
    body: JSON.stringify({ channel, code }),
  }).then((response) => ({
    ...response,
    success: true,
  }));
}

function channelPairingCopy(action, t) {
  return {
    title: action?.title || t("pairing.title"),
    instructions: action?.instructions || t("pairing.instructions"),
    placeholder:
      action?.input_placeholder || action?.code_placeholder || t("pairing.placeholder"),
    submitLabel: action?.submit_label || t("pairing.approve"),
    successMessage: action?.success_message || t("pairing.success"),
    errorMessage: action?.error_message || t("pairing.error"),
  };
}

function channelPairingError(error, fallback) {
  return error?.payload?.error || error?.payload?.message || error?.message || fallback;
}
