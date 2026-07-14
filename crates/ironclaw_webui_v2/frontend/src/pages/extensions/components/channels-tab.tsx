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

export function isOauthConnection(connection) {
  return connection?.strategy === "oauth";
}

function configurePayload(item) {
  return {
    packageRef: item?.package_ref,
    displayName: item?.display_name || packageId(item),
    surfaces: item?.surfaces,
    active: item?.active,
    authenticated: item?.authenticated,
    needs_setup: item?.needs_setup,
    activationStatus: item?.activation_status,
    onboardingState: item?.onboarding_state,
  };
}

export function OAuthChannelConnectionSection({ item, connection, onConfigure, isBusy }) {
  const instructions = String(connection?.instructions || "").trim();
  const submitLabel = String(connection?.submit_label || "").trim();
  if (!instructions && !submitLabel) return null;
  return (
    <div className="rounded-[14px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3">
      {instructions &&
      (
        <p className="text-xs leading-5 text-[var(--v2-text-muted)]">
          {instructions}
        </p>
      )}
      {submitLabel &&
      (
        <div className="mt-3 flex justify-end">
          <button
            type="button"
            disabled={isBusy || !onConfigure}
            onClick={() => onConfigure?.(configurePayload(item))}
            className="v2-button rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-3 py-1.5 text-xs font-medium text-[var(--v2-text-strong)] hover:bg-[var(--v2-surface-muted)] disabled:cursor-not-allowed disabled:opacity-50"
          >
            {submitLabel}
          </button>
        </div>
      )}
    </div>
  );
}

export function ChannelConnectSections({ item, onConfigure, isBusy }) {
  const connection = channelConnection(item);
  const sections = [];
  if (isSlackPackage(item)) {
    // Operator-only Slack workspace setup + channel routing. The section
    // self-gates on the operator-scoped setup endpoint, so non-operators see
    // nothing here. The typed OAuth connection copy below stays driven by the
    // extension surface instead of hardcoded Slack-specific onboarding text.
    sections.push(<SlackAdminManagedSection key="slack-admin" />);
  }
  if (isOauthConnection(connection)) {
    const channel = connection.channel || packageId(item);
    sections.push(
      <OAuthChannelConnectionSection
        key={`oauth-${channel}`}
        item={item}
        connection={connection}
        onConfigure={onConfigure}
        isBusy={isBusy}
      />
    );
  } else if (!isSlackPackage(item) && isInboundProofCodeConnection(connection)) {
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
                return (
                  <div key={packageId(ch)} className="flex flex-col gap-3">
                    <ExtensionCard
                      ext={ch}
                      onActivate={onActivate}
                      onConfigure={onConfigure}
                      onRemove={onRemove}
                      isBusy={isBusy}
                    />
                    <ChannelConnectSections
                      item={ch}
                      onConfigure={onConfigure}
                      isBusy={isBusy}
                    />
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
