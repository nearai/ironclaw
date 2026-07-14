import { Badge } from "../../../design-system/badge";
import { Card } from "../../../design-system/card";
import { useT } from "../../../lib/i18n";
import { useChannels } from "../hooks/useChannels";
import { matchesSearch } from "../lib/settings-search";
import { SettingsSearchEmpty } from "./settings-search-empty";

function packageId(item) {
  return item?.package_ref?.id || "";
}

function ExtensionChannelCard({ channel = null, registryEntry }) {
  const t = useT();
  const name =
    registryEntry?.display_name ||
    channel?.display_name ||
    packageId(channel) ||
    packageId(registryEntry) ||
    t("common.unknown");
  const desc = registryEntry?.description || channel?.description || "";
  const isInstalled = Boolean(channel);
  const state = channel?.onboarding_state || "setup_required";

  const toneMap = {
    ready: "positive",
    auth_required: "warning",
    pairing_required: "warning",
    setup_required: "muted",
  };
  const labelMap = {
    ready: t("channels.ready"),
    auth_required: t("channels.authNeeded"),
    pairing_required: t("channels.pairing"),
    setup_required: t("channels.setup"),
  };

  return (
    <div
      className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]">{name}</span>
          {isInstalled
            ? (<Badge
                tone={toneMap[state] || "muted"}
                label={labelMap[state] || state}
                size="sm"
              />)
            : (<Badge
                tone="muted"
                label={t("channels.available")}
                size="sm"
              />)}
        </div>
        <div className="mt-1 text-xs text-[var(--v2-text-muted)]">{desc}</div>
      </div>
    </div>
  );
}

function deriveVisibleChannelGroups({
  channels,
  channelRegistry,
  searchQuery,
  t,
}) {
  const installedIds = new Set(channels.map((channel) => packageId(channel)));
  const visibleChannels = channels.filter((channel) =>
    matchesSearch(searchQuery, [
      t("channels.messaging"),
      packageId(channel),
      channel.display_name,
      channel.description,
      channel.onboarding_state,
    ])
  );
  const availableRegistry = channelRegistry
    .filter((entry) => !installedIds.has(packageId(entry)))
    .filter((entry) =>
      matchesSearch(searchQuery, [
        t("channels.messaging"),
        packageId(entry),
        entry.display_name,
        entry.description,
      ])
    );

  return {
    visibleChannels,
    availableRegistry,
  };
}

export function ChannelsTab({ searchQuery = "" }) {
  const t = useT();
  const {
    channels,
    channelRegistry,
    isLoading,
  } = useChannels();

  if (isLoading) {
    return (
      <div className="space-y-5">
        <Card padding="md">
          <div className="mb-4 h-3 w-28 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
          {[1, 2, 3].map(
            (i) => (
              <div
                key={i}
                className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0"
              >
                <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                <div className="h-6 w-16 animate-pulse rounded-full bg-[var(--v2-surface-muted)]" />
              </div>
            )
          )}
        </Card>
      </div>
    );
  }

  const {
    visibleChannels,
    availableRegistry,
  } = deriveVisibleChannelGroups({
    channels,
    channelRegistry,
    searchQuery,
    t,
  });

  if (
    visibleChannels.length === 0 &&
    availableRegistry.length === 0
  ) {
    return (<SettingsSearchEmpty query={searchQuery} />);
  }

  return (
    <div className="space-y-5">
      {(visibleChannels.length > 0 || availableRegistry.length > 0) &&
      (
        <Card padding="md">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            {t("channels.messaging")}
          </h3>
          {visibleChannels.map(
            (ch) => (
              <ExtensionChannelCard
                key={packageId(ch)}
                channel={ch}
                registryEntry={channelRegistry.find(
                  (entry) => packageId(entry) === packageId(ch)
                )}
              />
            )
          )}
          {availableRegistry.map(
            (entry) => (
              <ExtensionChannelCard key={packageId(entry)} registryEntry={entry} />
            )
          )}
        </Card>
      )}
    </div>
  );
}
