import { React, html } from "../../../lib/html.js";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "../../../design-system/button.js";
import { useT } from "../../../lib/i18n.js";
import { usePairing } from "../hooks/useExtensions.js";
import { pairingErrorMessage } from "../lib/pairing-errors.js";

const DEFAULT_PAIRING_I18N_KEYS = {
  title: "pairing.title",
  instructions: "pairing.instructions",
  placeholder: "pairing.placeholder",
  action: "pairing.approve",
  success: "pairing.success",
  error: "pairing.error",
  empty: "pairing.none",
};

export function PairingSection({
  channel,
  redeemFn,
  i18nKeys = DEFAULT_PAIRING_I18N_KEYS,
  queryKeys,
  copy,
  showPendingRequests = true,
}) {
  const t = useT();
  const customRedeem = typeof redeemFn === "function";
  const pairing = usePairing(channel, { enabled: !customRedeem });
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

  const handleApprove = React.useCallback(
    (code) => pairing.approve({ code }),
    [pairing.approve]
  );

  const handleManualSubmit = React.useCallback(() => {
    const trimmed = manualCode.trim();
    if (!trimmed) return;
    if (customRedeem) {
      redeemMutation.mutate({ code: trimmed });
    } else {
      pairing.approve({ code: trimmed });
      setManualCode("");
    }
  }, [customRedeem, manualCode, pairing.approve, redeemMutation]);

  const requests = customRedeem ? [] : pairing.requests;
  const isLoading = customRedeem ? false : pairing.isLoading;
  const isApproving = customRedeem ? redeemMutation.isPending : pairing.isApproving;
  const result = customRedeem
    ? redeemMutation.isSuccess ? redeemMutation.data : null
    : pairing.result;
  const error = customRedeem
    ? redeemMutation.isError ? redeemMutation.error : null
    : pairing.error;

  if (isLoading) {
    return html`
      <div className="mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4">
        <div className="v2-skeleton h-3 w-24 rounded" />
      </div>
    `;
  }

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

      ${showPendingRequests && requests.length > 0
        ? html`
            <div className="space-y-2">
              ${requests.map((req) => html`
                <div
                  key=${req.code || req.id}
                  className="flex items-center justify-between gap-3 rounded-md border border-white/[0.06] bg-white/[0.02] px-3 py-2"
                >
                  <div className="min-w-0">
                    <span className="font-mono text-sm text-iron-200">${req.code || req.id}</span>
                    ${req.label && html`
                      <span className="ml-2 text-xs text-iron-300">${req.label}</span>
                    `}
                  </div>
                  <${Button}
                    variant="secondary"
                    className="h-7 px-2.5 text-xs"
                    onClick=${() => handleApprove(req.code || req.id)}
                    disabled=${isApproving}
                  >
                    ${pairingCopy.action}
                  <//>
                </div>
              `)}
            </div>
          `
        : showPendingRequests &&
          html`<p className="text-xs text-iron-300">${t(i18nKeys.empty)}</p>`}
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
