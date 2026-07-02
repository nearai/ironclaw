import { StatusPill } from "../../../design-system/primitives.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { SlackAdminManagedSection } from "../../../components/slack-setup-panel.js";
import { SlackPairingSection } from "../../../components/slack-pairing-section.js";
import { ExtensionCard, RegistryCard } from "./extension-card.js";
import { PairingSection } from "./pairing-section.js";

function packageId(item) {
  return item.package_ref?.id || "";
}

export function isSlackPackage(item) {
  return packageId(item) === "slack";
}

export function isSlackAdminManagedAction(connectAction) {
  return connectAction?.channel === "slack" && connectAction.strategy === "admin_managed_channels";
}

export function isSlackInboundProofCodeAction(connectAction) {
  return connectAction?.channel === "slack" && connectAction.strategy === "inbound_proof_code";
}

export function findSlackConnectAction(connectableChannels) {
  return findSlackConnectActions(connectableChannels)[0] || null;
}

export function findSlackConnectActions(connectableChannels) {
  const channels = connectableChannels || [];
  const actions = [
    channels.find(isSlackAdminManagedAction),
    channels.find(isSlackInboundProofCodeAction),
  ].filter(Boolean);
  if (actions.length > 0) return actions;
  const fallback = channels.find((channel) => channel.channel === "slack");
  return fallback ? [fallback] : [];
}

export function SlackConnectActionSections({
  slackConnectAction,
  slackConnectActions,
}) {
  const actions =
    slackConnectActions || (slackConnectAction ? [slackConnectAction] : []);
  const sections = actions
    .map((action) => {
      if (isSlackAdminManagedAction(action)) {
        return html`<${SlackAdminManagedSection} action=${action.action} />`;
      }
      if (isSlackInboundProofCodeAction(action)) {
        return html`<${SlackPairingSection} action=${action.action} />`;
      }
      return null;
    })
    .filter(Boolean);
  return sections.length > 0
    ? html`<div className="space-y-3">${sections}</div>`
    : null;
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
            <${SlackConnectActionSections}
              slackConnectActions=${slackConnectActions}
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
              (ch) => html`
                <div key=${packageId(ch)} className="flex flex-col gap-3">
                  <${ExtensionCard}
                    ext=${ch}
                    onActivate=${onActivate}
                    onConfigure=${onConfigure}
                    onRemove=${onRemove}
                    isBusy=${isBusy}
                  />
                  ${isSlackPackage(ch) &&
                  html`<${SlackConnectActionSections}
                    slackConnectActions=${slackConnectActions}
                  />`}
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
