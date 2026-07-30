import { Card } from "@ironclaw/ui";
import { useT } from "../../../lib/i18n";
import { ExtensionCard, RegistryCard } from "./extension-card";
import type {
  ConfigureFocusHandler,
  InstallFocusHandler,
} from "../lib/focus-target";

function packageId(item) {
  return item?.package_ref?.id || "";
}

/**
 * @param {{
 *   channels?: any[];
 *   channelRegistry: any[];
 *   onConfigure: ConfigureFocusHandler<any>;
 *   onRemove: (extension: any) => void;
 *   onInstall: InstallFocusHandler;
 *   isBusy: boolean;
 * }} props
 */
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
        <Card className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[0.6875rem] font-semibold uppercase tracking-[0.16em] text-[var(--v2-accent-text)]">
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
        </Card>
      )}
      {channelRegistry.length > 0 &&
      (
        <Card className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[0.6875rem] font-semibold uppercase tracking-[0.16em] text-[var(--v2-accent-text)]">
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
        </Card>
      )}
    </div>
  );
}
