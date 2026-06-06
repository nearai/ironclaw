import { StatusPill } from "../../../design-system/primitives.js";
import { html } from "../../../lib/html.js";
import { ExtensionCard, RegistryCard } from "./extension-card.js";
import { PairingSection } from "./pairing-section.js";
import { redeemPairingCode } from "../lib/pairing-api.js";

const SLACK_PAIRING_I18N_KEYS = {
  title: "pairing.slackTitle",
  instructions: "pairing.slackInstructions",
  placeholder: "pairing.slackPlaceholder",
  action: "pairing.connect",
  success: "pairing.slackSuccess",
  error: "pairing.slackError",
  empty: "pairing.none",
};

const SLACK_PAIRING_QUERY_KEYS = [
  ["extensions"],
  ["pairing", "slack"],
  ["connectable-channels"],
];

function packageId(item) {
  return item.package_ref?.id || "";
}

export function isSlackChannelEnabled(enabledChannels) {
  return ["slack", "slack_v2", "slack-v2"].some((channel) =>
    enabledChannels.includes(channel)
  );
}

export function ChannelsTab({
  status,
  channels,
  connectableChannels,
  channelRegistry,
  onActivate,
  onConfigure,
  onRemove,
  onInstall,
  isBusy,
}) {
  const enabledChannels = status.enabled_channels || [];
  const slackEnabled = isSlackChannelEnabled(enabledChannels);
  const slackConnectAction = connectableChannels?.find((channel) => channel.channel === "slack");
  const slackStatusLabel = slackEnabled ? "on" : slackConnectAction ? "connect" : "off";
  const slackStatusTone = slackEnabled ? "success" : slackConnectAction ? "info" : "muted";

  return html`
    <div className="space-y-5">
      <div className="v2-panel rounded-[18px] p-5 sm:p-6">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
        >
          Built-in channels
        </h3>
        <${BuiltinRow}
          name="Web Gateway"
          description="Browser-based chat with SSE streaming"
          enabled=${true}
          detail=${"SSE: " +
          (status.sse_connections || 0) +
          " · WS: " +
          (status.ws_connections || 0)}
        />
        <${BuiltinRow}
          name="HTTP Webhook"
          description="Inbound webhook endpoint for external integrations"
          enabled=${enabledChannels.includes("http")}
          detail="ENABLE_HTTP=true"
        />
        <${BuiltinRow}
          name="Slack"
          description="Tenant app channel for DMs and app mentions"
          enabled=${slackEnabled}
          statusLabel=${slackStatusLabel}
          statusTone=${slackStatusTone}
          detail="Tenant Slack app install"
        >
          ${slackConnectAction &&
          html`<${PairingSection}
            channel="slack"
            redeemFn=${redeemPairingCode}
            i18nKeys=${SLACK_PAIRING_I18N_KEYS}
            copy=${slackConnectAction.action}
            queryKeys=${SLACK_PAIRING_QUERY_KEYS}
            showPendingRequests=${false}
          />`}
        <//>
        <${BuiltinRow}
          name="CLI"
          description="Terminal interface with TUI or simple REPL"
          enabled=${enabledChannels.includes("cli")}
          detail="ironclaw run --cli"
        />
        <${BuiltinRow}
          name="REPL"
          description="Minimal read-eval-print loop for testing"
          enabled=${enabledChannels.includes("repl")}
          detail="ironclaw run --repl"
        />
      </div>

      ${channels.length > 0 &&
      html`
        <div className="v2-panel rounded-[18px] p-5 sm:p-6">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            Messaging channels
          </h3>
          <div className="grid grid-cols-1 gap-4">
            ${channels.map(
              (ch) => html`
                <div key=${packageId(ch)} className="flex flex-col gap-3">
                  <${ExtensionCard}
                    ext=${ch}
                    onActivate=${onActivate}
                    onConfigure=${onConfigure}
                    onRemove=${onRemove}
                    isBusy=${isBusy}
                  />
                  ${(ch.onboarding_state === "pairing_required" ||
                    ch.onboarding_state === "pairing") &&
                  html` <${PairingSection} channel=${packageId(ch)} /> `}
                </div>
              `
            )}
          </div>
        </div>
      `}
      ${channelRegistry.length > 0 &&
      html`
        <div className="v2-panel rounded-[18px] p-5 sm:p-6">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            Available channels
          </h3>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
            ${channelRegistry.map(
              (entry) => html`
                <${RegistryCard}
                  key=${packageId(entry)}
                  entry=${entry}
                  onInstall=${onInstall}
                  isBusy=${isBusy}
                />
              `
            )}
          </div>
        </div>
      `}
    </div>
  `;
}

function BuiltinRow({
  name,
  description,
  enabled,
  detail,
  children,
  statusLabel = enabled ? "on" : "off",
  statusTone = enabled ? "success" : "muted",
}) {
  return html`
    <div
      className="border-t border-white/[0.06] py-4 first:border-0 first:pt-0"
    >
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-sm font-medium text-iron-200">${name}</span>
            <${StatusPill}
              tone=${statusTone}
              label=${statusLabel}
            />
          </div>
          <div className="mt-1 text-xs text-iron-300">${description}</div>
          ${detail &&
          html`<div className="mt-1 font-mono text-[11px] text-iron-700">
            ${detail}
          </div>`}
        </div>
      </div>
      ${children}
    </div>
  `;
}
