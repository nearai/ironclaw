import { React, html } from "../../../lib/html.js";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "../../../design-system/button.js";
import { useT } from "../../../lib/i18n.js";
import { pairingErrorMessage } from "../lib/pairing-errors.js";

const DEFAULT_PAIRING_I18N_KEYS = {
  title: "pairing.title",
  instructions: "pairing.instructions",
  placeholder: "pairing.placeholder",
  action: "pairing.approve",
  success: "pairing.success",
  error: "pairing.error",
};

// Paste-a-code pairing for a connectable channel. Redemption always goes through
// the caller-provided `redeemFn` (the mounted v2 redeem endpoint). Reborn v2 has
// no admin pending-pairing-request queue, so there is no legacy approve path.
export function PairingSection({
  channel,
  redeemFn,
  i18nKeys = DEFAULT_PAIRING_I18N_KEYS,
  queryKeys,
  copy,
}) {
  const t = useT();
  const queryClient = useQueryClient();
  const [manualCode, setManualCode] = React.useState("");
  const pairingCopy = resolvePairingCopy(t, i18nKeys, copy);

  const redeemMutation = useMutation({
    mutationFn: ({ code }) => redeemFn(channel, code),
    onSuccess: () => {
      setManualCode("");
      for (const queryKey of queryKeys || [["pairing", channel], ["extensions"]]) {
        queryClient.invalidateQueries({ queryKey });
      }
    },
  });

  const handleManualSubmit = React.useCallback(() => {
    const trimmed = manualCode.trim();
    if (!trimmed) return;
    redeemMutation.mutate({ code: trimmed });
  }, [manualCode, redeemMutation]);

  const isApproving = redeemMutation.isPending;
  const result = redeemMutation.isSuccess ? redeemMutation.data : null;
  const error = redeemMutation.isError ? redeemMutation.error : null;

  return html`
    <div className="mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4">
      <h4 className="mb-3 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
        ${pairingCopy.title}
      </h4>
      <p className="mb-4 text-xs leading-5 text-iron-300">${pairingCopy.instructions}</p>

      <div className="mb-3 flex flex-col gap-2 sm:flex-row sm:items-center">
        <input
          type="text"
          value=${manualCode}
          onChange=${(e) => setManualCode(e.target.value)}
          onKeyDown=${(e) => e.key === "Enter" && handleManualSubmit()}
          placeholder=${pairingCopy.placeholder}
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <${Button}
          variant="secondary"
          className="h-9 shrink-0 px-3 text-xs"
          onClick=${handleManualSubmit}
          disabled=${isApproving || !manualCode.trim()}
        >
          ${pairingCopy.action}
        <//>
      </div>

      ${result?.success &&
      html`<p className="mb-3 text-xs text-emerald-300">
        ${result.message || pairingCopy.success}
      </p>`}
      ${result && !result.success &&
      html`<p className="mb-3 text-xs text-red-300">
        ${result.message || pairingCopy.error}
      </p>`}
      ${error &&
      html`<p className="mb-3 text-xs text-red-300">
        ${pairingErrorMessage(error, pairingCopy.error)}
      </p>`}
    </div>
  `;
}

function resolvePairingCopy(t, i18nKeys, copy) {
  return {
    title: copy?.title || t(i18nKeys.title),
    instructions: copy?.instructions || t(i18nKeys.instructions),
    placeholder:
      copy?.input_placeholder || copy?.code_placeholder || t(i18nKeys.placeholder),
    action: copy?.submit_label || t(i18nKeys.action),
    success: copy?.success_message || t(i18nKeys.success),
    error: copy?.error_message || t(i18nKeys.error),
  };
}
