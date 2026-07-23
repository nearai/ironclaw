import { useT } from "../../../lib/i18n";
import { ExtensionCard, RegistryCard } from "./extension-card";

function packageId(item) {
  return item?.package_ref?.id || "";
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
              (ch) => (
                <ExtensionCard
                  key={packageId(ch)}
                  ext={ch}
                  onConfigure={onConfigure}
                  onRemove={onRemove}
                  isBusy={isBusy}
                />
              )
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
