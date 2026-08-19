import { DeviceLinkPanel } from "../../../components/device-link-panel";
import { notifyChannelConnected } from "../../../lib/channel-connection-events";
import { useExtensionSetup } from "../../extensions/hooks/useExtensions";
import { deviceLinkSetupSecret } from "../../extensions/lib/extensions-schema";
import { useT } from "../../../lib/i18n";

/**
 * Lazy device-link body for an install-result onboarding card.
 *
 * The setup contract is the authority for the credential provider. Keeping
 * that lookup here avoids conflating an extension id with a vendor id, while
 * the lazy boundary prevents extension-registry code from inflating every
 * ordinary chat load.
 */
export function OnboardingDeviceLinkPanel({ onboarding, displayName, errorMessage }) {
  const t = useT();
  const setup = useExtensionSetup(onboarding?.extensionName || null);
  const deviceLinkSecret = deviceLinkSetupSecret(setup.secrets);

  if (setup.isLoading) {
    return <p className="mt-3 text-sm text-iron-400">{t("common.loading")}</p>;
  }
  if (setup.error || !deviceLinkSecret) {
    return (
      <p role="alert" className="mt-3 text-xs leading-5 text-red-300">
        {errorMessage}
      </p>
    );
  }

  return (
    <DeviceLinkPanel
      provider={deviceLinkSecret.provider}
      extensionName={onboarding.extensionName}
      displayName={displayName || onboarding.extensionName}
      onCompleted={() =>
        notifyChannelConnected({
          channel: onboarding.extensionName,
          source: "chat-device-link",
        })}
    />
  );
}
