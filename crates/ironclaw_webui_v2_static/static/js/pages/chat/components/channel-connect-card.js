import { SlackPairingSection } from "../../../components/slack-pairing-section.js";
import { Icon } from "../../../design-system/icons.js";
import { html } from "../../../lib/html.js";

export function isSlackStrategy(connectAction, strategy) {
  return connectAction?.channel === "slack" && connectAction.strategy === strategy;
}

export function isConnectedChannel(connectAction) {
  return connectAction?.connection_status === "connected";
}

export function shouldRenderSlackPairingSection(connectAction) {
  return !isConnectedChannel(connectAction) && isSlackStrategy(connectAction, "inbound_proof_code");
}

export function ChannelConnectCard({ connectAction, onDismiss }) {
  if (!connectAction) return null;
  const channel = connectAction.channel;
  const titleVerb = isConnectedChannel(connectAction) ? "Connected" : "Connect";

  return html`
    <div className="rounded-[16px] border border-white/[0.06] bg-white/[0.02] p-3">
      <div className="mb-2 flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
            ${titleVerb} ${connectAction.display_name || channel}
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

      ${isConnectedChannel(connectAction)
        ? html`
            <div className="rounded-xl border border-white/[0.06] bg-white/[0.02] p-4 text-xs leading-5 text-iron-300">
              ${connectAction.display_name || channel} is connected.
            </div>
          `
        : shouldRenderSlackPairingSection(connectAction)
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
