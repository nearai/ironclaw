import { SlackPairingSection } from "../../../components/slack-pairing-section.js";
import { Icon } from "../../../design-system/icons.js";
import { html } from "../../../lib/html.js";

export function ChannelConnectCard({ connectAction, onDismiss }) {
  if (!connectAction) return null;
  const channel = connectAction.channel;

  return html`
    <div className="rounded-[16px] border border-white/[0.06] bg-white/[0.02] p-3">
      <div className="mb-2 flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
            Connect ${connectAction.display_name || channel}
          </div>
        </div>
        ${onDismiss &&
        html`
          <button
            type="button"
            aria-label="Dismiss connect action"
            onClick=${onDismiss}
            className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-iron-400 hover:bg-white/[0.04] hover:text-iron-100"
          >
            <${Icon} name="close" className="h-4 w-4" />
          </button>
        `}
      </div>

      ${channel === "slack" && connectAction.strategy === "inbound_proof_code"
        ? html`<${SlackPairingSection} action=${connectAction.action} />`
        : html`
            <div className="rounded-xl border border-white/[0.06] bg-white/[0.02] p-4 text-xs leading-5 text-iron-300">
              ${connectAction.action?.instructions ||
              "This channel exposes a connect action, but the WebUI has no renderer for its strategy yet."}
            </div>
          `}
    </div>
  `;
}
