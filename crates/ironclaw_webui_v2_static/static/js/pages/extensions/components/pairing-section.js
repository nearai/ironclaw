import { React, html } from "../../../lib/html.js";
import { StatusPill } from "../../../design-system/primitives.js";
import { Button } from "../../../design-system/button.js";
import { useT } from "../../../lib/i18n.js";
import { usePairing } from "../hooks/useExtensions.js";

export function PairingSection({ channel }) {
  const t = useT();
  const { requests, isLoading, approve, isApproving } = usePairing(channel);
  const [manualCode, setManualCode] = React.useState("");

  const handleApprove = React.useCallback(
    (code) => approve({ code }),
    [approve]
  );

  const handleManualSubmit = React.useCallback(() => {
    const trimmed = manualCode.trim();
    if (trimmed) {
      approve({ code: trimmed });
      setManualCode("");
    }
  }, [manualCode, approve]);

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
        ${t("pairing.title")}
      </h4>

      <div className="mb-4 flex items-center gap-2">
        <input
          type="text"
          value=${manualCode}
          onChange=${(e) => setManualCode(e.target.value)}
          onKeyDown=${(e) => e.key === "Enter" && handleManualSubmit()}
          placeholder=${t("pairing.placeholder")}
          className="h-9 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <${Button}
          variant="secondary"
          className="h-9 px-3 text-xs"
          onClick=${handleManualSubmit}
          disabled=${isApproving || !manualCode.trim()}
        >
          ${t("pairing.approve")}
        <//>
      </div>

      ${requests.length > 0
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
                    ${t("pairing.approve")}
                  <//>
                </div>
              `)}
            </div>
          `
        : html`<p className="text-xs text-iron-300">${t("pairing.none")}</p>`}
    </div>
  `;
}
