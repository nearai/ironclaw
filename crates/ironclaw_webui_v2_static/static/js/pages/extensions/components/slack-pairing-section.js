import { React, html } from "../../../lib/html.js";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "../../../design-system/button.js";
import { useT } from "../../../lib/i18n.js";
import { redeemSlackPairingCode } from "../lib/slack-pairing-api.js";

export function SlackPairingSection() {
  const t = useT();
  const queryClient = useQueryClient();
  const redeemMutation = useMutation({
    mutationFn: ({ code }) => redeemSlackPairingCode(code),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["extensions"] });
      queryClient.invalidateQueries({ queryKey: ["pairing", "slack"] });
    },
  });
  const [manualCode, setManualCode] = React.useState("");

  const submit = () => {
    const code = manualCode.trim();
    if (!code) return;
    redeemMutation.mutate({ code });
    setManualCode("");
  };

  return html`
    <div className="mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4">
      <h4 className="mb-3 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
        ${t("pairing.slackTitle")}
      </h4>
      <p className="mb-4 text-xs leading-5 text-iron-300">
        ${t("pairing.slackInstructions")}
      </p>

      <div className="mb-3 flex flex-col gap-2 sm:flex-row sm:items-center">
        <input
          type="text"
          value=${manualCode}
          onChange=${(event) => setManualCode(event.target.value)}
          onKeyDown=${(event) => event.key === "Enter" && submit()}
          placeholder=${t("pairing.slackPlaceholder")}
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <${Button}
          variant="secondary"
          className="h-9 shrink-0 px-3 text-xs"
          onClick=${submit}
          disabled=${redeemMutation.isPending || !manualCode.trim()}
        >
          ${t("pairing.connect")}
        <//>
      </div>

      ${redeemMutation.isSuccess &&
      html`<p className="text-xs text-emerald-300">
        ${redeemMutation.data?.message || t("pairing.slackSuccess")}
      </p>`}
      ${redeemMutation.isError &&
      html`<p className="text-xs text-red-300">
        ${slackPairingError(redeemMutation.error, t("pairing.slackError"))}
      </p>`}
    </div>
  `;
}

function slackPairingError(error, fallback) {
  return error?.payload?.error || error?.payload?.message || error?.message || fallback;
}
