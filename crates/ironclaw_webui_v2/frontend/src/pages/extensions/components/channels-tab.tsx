import { useT } from "../../../lib/i18n";
import { SlackAdminManagedSection } from "../../../components/slack-setup-panel";
import { ExtensionCard, RegistryCard } from "./extension-card";
import { PairingSection } from "./pairing-section";
import { redeemPairingCode } from "../lib/pairing-api";

function packageId(item) {
  return item?.package_ref?.id || "";
}

export function isSlackPackage(item) {
  return packageId(item) === "slack";
}

// Channel discovery is extension-surface data: an extension's `surfaces`
// carry a typed `channel` entry with direction (inbound/outbound), the
// caller's connection state, and the connect affordance. There is no separate
// connectable-channel registry.
export function channelSurface(item) {
  const surfaces = item?.surfaces || [];
  return surfaces.find((surface) => surface?.kind === "channel") || null;
}

export function channelConnection(item) {
  return channelSurface(item)?.connection || null;
}

export function isInboundProofCodeConnection(connection) {
  return connection?.strategy === "inbound_proof_code";
}

export function ChannelConnectSections({ item }) {
  const connection = channelConnection(item);
  const sections = [];
  if (isSlackPackage(item)) {
    // Operator-only Slack workspace setup + channel routing. The section
    // self-gates on the operator-scoped setup endpoint, so non-operators see
    // nothing here; the user OAuth connect lives in the configure modal.
    sections.push(<SlackAdminManagedSection key="slack-admin" />);
  } else if (isInboundProofCodeConnection(connection)) {
    const pairingChannel = connection.channel || packageId(item);
    sections.push(
      <PairingSection
        key={`pairing-${pairingChannel}`}
        channel={pairingChannel}
        copy={connection}
        redeemFn={redeemPairingCode}
        showPendingRequests={false}
        queryKeys={[["extensions"], ["pairing", pairingChannel]]}
      />
    );
  }
  return sections.length > 0
    ? (<div className="space-y-3">{sections}</div>)
    : null;
}

export function ChannelsTab({
  channels,
  channelRegistry,
  onActivate,
  onConfigure,
  onRemove,
  onInstall,
  isBusy,
}) {
  const t = useT();
  const installedChannels = channels || [];

  return (
    <div className="space-y-5">
      {installedChannels.length > 0 &&
      (
        <div className="v2-panel rounded-[18px] p-5 sm:p-6">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            {t("channels.messaging")}
          </h3>
          <div className="grid grid-cols-1 gap-4">
            {installedChannels.map(
              (ch) => {
                const connection = channelConnection(ch);
                const pairingHandledBySurface =
                  isInboundProofCodeConnection(connection);
                return (
                  <div key={packageId(ch)} className="flex flex-col gap-3">
                    <ExtensionCard
                      ext={ch}
                      onActivate={onActivate}
                      onConfigure={onConfigure}
                      onRemove={onRemove}
                      isBusy={isBusy}
                    />
                    <ChannelConnectSections item={ch} />
                    {!isSlackPackage(ch) &&
                    !pairingHandledBySurface &&
                    (ch.onboarding_state === "pairing_required" ||
                      ch.onboarding_state === "pairing") &&
                    ( <PairingSection
                      channel={packageId(ch)}
                      redeemFn={redeemPairingCode}
                    /> )}
                  </div>
                );
              }
            )}
          </div>
        </div>
      )}
      {channelRegistry.length > 0 &&
      (
        <div className="v2-panel rounded-[18px] p-5 sm:p-6">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            {t("channels.availableChannels")}
          </h3>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
            {channelRegistry.map(
              (entry) => (
                <RegistryCard
                  key={packageId(entry)}
                  entry={entry}
                  onInstall={onInstall}
                  isBusy={isBusy}
                />
              )
            )}
          </div>
        </div>
      )}
    </div>
  );
}
