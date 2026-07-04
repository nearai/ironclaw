import { StatusPill } from "../../../design-system/primitives.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { SlackAdminManagedSection } from "../../../components/slack-setup-panel.js";
import { SlackPairingSection } from "../../../components/slack-pairing-section.js";
import { ExtensionCard, RegistryCard } from "./extension-card.js";
import { PairingSection } from "./pairing-section.js";
import { redeemPairingCode } from "../lib/pairing-api.js";

function packageId(item) {
  return item?.package_ref?.id || "";
}

export function isSlackPackage(item) {
  return packageId(item) === "slack";
}

export function isAdminManagedChannelsAction(connectAction) {
  return connectAction?.strategy === "admin_managed_channels";
}

export function isInboundProofCodeAction(connectAction) {
  return connectAction?.strategy === "inbound_proof_code";
}

export function isSlackAdminManagedAction(connectAction) {
  return connectAction?.channel === "slack" && isAdminManagedChannelsAction(connectAction);
}

export function isSlackInboundProofCodeAction(connectAction) {
  return connectAction?.channel === "slack" && isInboundProofCodeAction(connectAction);
}

export function findSlackConnectAction(connectableChannels) {
  return findSlackConnectActions(connectableChannels)[0] || null;
}

export function findSlackConnectActions(connectableChannels) {
  return connectActionsForChannel(connectableChannels, "slack");
}

export function connectActionsForPackage(connectableChannels, item) {
  return connectActionsForChannel(connectableChannels, packageId(item));
}

export function connectActionsForChannel(connectableChannels, channel) {
  if (!channel) return [];
  const channels = connectableChannels || [];
  const actions = [
    channels.find(
      (connectAction) =>
        connectAction?.channel === channel && isAdminManagedChannelsAction(connectAction)
    ),
    channels.find(
      (connectAction) =>
        connectAction?.channel === channel && isInboundProofCodeAction(connectAction)
    ),
  ].filter(Boolean);
  if (actions.length > 0) return actions;
  const fallback = channels.find((connectAction) => connectAction?.channel === channel);
  return fallback ? [fallback] : [];
}

export function ChannelConnectActionSections({
  connectAction,
  connectActions,
}) {
  const actions =
    connectActions || (connectAction ? [connectAction] : []);
  const sections = actions
    .map((action) => {
      if (isSlackAdminManagedAction(action)) {
        return html`<${SlackAdminManagedSection} action=${action.action} />`;
      }
      if (isSlackInboundProofCodeAction(action)) {
        return html`<${SlackPairingSection} action=${action.action} />`;
      }
      if (isInboundProofCodeAction(action)) {
        return html`
          <${PairingSection}
            channel=${action.channel}
            copy=${action.action}
            redeemFn=${redeemPairingCode}
            showPendingRequests=${false}
            queryKeys=${[
              ["extensions"],
              ["connectable-channels"],
              ["pairing", action.channel],
            ]}
          />
        `;
      }
      return null;
    })
    .filter(Boolean);
  return sections.length > 0
    ? html`<div className="space-y-3">${sections}</div>`
    : null;
}

export function SlackConnectActionSections({
  slackConnectAction,
  slackConnectActions,
}) {
  return ChannelConnectActionSections({
    connectAction: slackConnectAction,
    connectActions: slackConnectActions,
  });
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
  const t = useT();
  const installedChannels = channels || [];
  const enabledChannels = status.enabled_channels || [];
  const slackConnectActions = findSlackConnectActions(connectableChannels);
  const hasInstalledSlackPackage = installedChannels.some(isSlackPackage);
  const showBuiltinSlackConnectActions =
    slackConnectActions.length > 0 && !hasInstalledSlackPackage;

  return html`
    <div className="space-y-5">
      <div className="v2-panel rounded-[18px] p-5 sm:p-6">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
        >
          ${t("channels.builtIn")}
        </h3>
        <${BuiltinRow}
          name=${t("channels.webGateway")}
          description=${t("channels.webGatewayDesc")}
          enabled=${true}
          detail=${"SSE: " +
          (status.sse_connections || 0) +
          " · WS: " +
          (status.ws_connections || 0)}
        />
        <${BuiltinRow}
          name=${t("channels.httpWebhook")}
          description=${t("channels.httpWebhookDesc")}
          enabled=${enabledChannels.includes("http")}
          detail="ENABLE_HTTP=true"
        />
        <${BuiltinRow}
          name=${t("channels.cli")}
          description=${t("channels.cliDesc")}
          enabled=${enabledChannels.includes("cli")}
          detail="ironclaw run --cli"
        />
        <${BuiltinRow}
          name=${t("channels.repl")}
          description=${t("channels.replDesc")}
          enabled=${enabledChannels.includes("repl")}
          detail="ironclaw run --repl"
        />
        ${showBuiltinSlackConnectActions &&
        html`
          <${BuiltinRow}
            name=${t("channels.slack")}
            description=${t("channels.slackDesc")}
            enabled=${false}
            statusLabel=${t("channels.setup")}
            statusTone="muted"
            detail=${t("channels.slackDetail")}
          >
            <${ChannelConnectActionSections}
              connectActions=${slackConnectActions}
            />
          </${BuiltinRow}>
        `}
      </div>

      ${installedChannels.length > 0 &&
      html`
        <div className="v2-panel rounded-[18px] p-5 sm:p-6">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            ${t("channels.messaging")}
          </h3>
          <div className="grid grid-cols-1 gap-4">
            ${installedChannels.map(
              (ch) => {
                const connectActions = connectActionsForPackage(connectableChannels, ch);
                const pairingHandledByConnectAction =
                  connectActions.some(isInboundProofCodeAction);
                return html`
                  <div key=${packageId(ch)} className="flex flex-col gap-3">
                    <${ExtensionCard}
                      ext=${ch}
                      onActivate=${onActivate}
                      onConfigure=${onConfigure}
                      onRemove=${onRemove}
                      isBusy=${isBusy}
                    />
                    ${connectActions.length > 0 &&
                    html`<${ChannelConnectActionSections}
                      connectActions=${connectActions}
                    />`}
                    ${!pairingHandledByConnectAction &&
                    (ch.onboarding_state === "pairing_required" ||
                      ch.onboarding_state === "pairing") &&
                    html` <${PairingSection}
                      channel=${packageId(ch)}
                      redeemFn=${redeemPairingCode}
                    /> `}
                  </div>
                `;
              }
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
            ${t("channels.availableChannels")}
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
