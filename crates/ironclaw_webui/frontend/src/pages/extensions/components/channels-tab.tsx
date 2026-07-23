import { useT } from "../../../lib/i18n";
import {
  channelConnection,
  isInboundProofCodeConnection,
} from "../lib/extensions-schema";
import { ExtensionCard, RegistryCard } from "./extension-card";
import { PairingSection } from "./pairing-section";
import { redeemPairingCode } from "../lib/pairing-api";

function packageId(item) {
  return item?.package_ref?.id || "";
}

// Every channel package renders the same generic sections: the connect
// section derives from the surface `connection` strategy, and the configure
// affordance from caller-scoped setup credentials in the configure modal. OAuth
// connections (and channels without a connect affordance) render nothing
// here — OAuth connect lives in the configure modal.
export function ChannelConnectSections({ item }) {
  const connection = channelConnection(item);
  const sections = [];
  if (isInboundProofCodeConnection(connection)) {
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
                      onConfigure={onConfigure}
                      onRemove={onRemove}
                      isBusy={isBusy}
                    />
                    <ChannelConnectSections item={ch} />
                    {!pairingHandledBySurface &&
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
