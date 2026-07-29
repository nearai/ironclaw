// @ts-nocheck
/**
 * The DMK adapter (attested-signing §D4).
 *
 * Binds `@ledgerhq/device-management-kit` + `device-transport-kit-web-hid` to
 * the four-method device port the ceremony is written against. It holds no
 * policy: every rule about whether signing is permitted lives in the ceremony,
 * deliberately outside the vendor SDK.
 *
 * ## Egress lockdown
 *
 * The context module ships four remote endpoints, all `*.ledger.com`:
 *
 * | Endpoint | Default | What it is |
 * |---|---|---|
 * | `cal` | `global.api.prd.ledger.com/cal/v1` | ERC-7730 descriptors |
 * | `web3checks` | `global.api.prd.ledger.com/transaction-checks/v3` | transaction screening |
 * | `metadataServiceDomain` | `nft.api.live.ledger.com` | NFT metadata |
 * | `reporter` | `blind-signing.api.ledger.com/ingest/v1` | **blind-signing telemetry** |
 *
 * Every one is repointed here. The first is our same-origin descriptor proxy
 * (§D3) — the SPA's CSP forbids remote origins, so the default would be blocked
 * anyway; pointing it at the proxy is what makes descriptors work at all. The
 * other three are pinned to an inert same-origin path because *silently
 * blocked* and *deliberately disabled* are different things: the first leaves a
 * request that starts working the day someone loosens the CSP.
 *
 * The reporter deserves naming: it exists to phone home about blind-signing
 * events. We do not blind-sign — a transaction without a descriptor never
 * reaches the device — so it has nothing legitimate to report, and an
 * un-pinned telemetry endpoint in a signing bundle is exactly what the
 * `@sentry/minimal` override already removed once.
 *
 * ## Unverified against hardware
 *
 * The state and error mappings below are exported and unit-tested against
 * DMK-shaped values, so their *logic* is covered. Whether DMK actually emits
 * these shapes for a locked device, a wrong app, or a mid-sign unplug can only
 * be established against a real Ledger — see
 * `docs/internal/ledger-clear-signing-manual-qa.md`. Treat this file as
 * unverified until that checklist is executed.
 */

import { DEVICE_STATUS, DeviceDisconnectedError, DeviceRejectedError } from "./device-port";

/** Same-origin path that resolves nothing, for endpoints we refuse to use. */
const DISABLED_ENDPOINT = "/api/webchat/v2/_disabled";

/** Our same-origin descriptor proxy for a given intent (§D3). */
export function descriptorProxyUrl(intentId) {
  return `/api/webchat/v2/intents/${encodeURIComponent(intentId)}/signing-context`;
}

/**
 * Map a DMK session state onto a port status.
 *
 * Exported for testing: this mapping decides which of five different user
 * instructions is shown, and getting it wrong tells someone to unlock a device
 * that is actually running the wrong app.
 */
export function mapSessionState(sessionState, expectedApp = "Ethereum") {
  if (!sessionState) return DEVICE_STATUS.disconnected;

  const status = sessionState.deviceStatus;
  if (status === "LOCKED") return DEVICE_STATUS.locked;
  if (status === "NOT CONNECTED") return DEVICE_STATUS.disconnected;

  // A device can be CONNECTED and still be sitting on the dashboard or in
  // another app. `currentApp` is only present once the session is ready.
  const currentApp = sessionState.currentApp?.name ?? sessionState.currentApp;
  if (currentApp && currentApp !== expectedApp) return DEVICE_STATUS.wrongApp;
  if (!currentApp) return DEVICE_STATUS.wrongApp;

  if (status === "CONNECTED" || status === "BUSY") return DEVICE_STATUS.ready;
  return DEVICE_STATUS.disconnected;
}

/**
 * Translate a DMK error into the port's error vocabulary.
 *
 * A user declining on the device and a device being unplugged are different
 * events with different follow-ups, and DMK reports both as errors on the same
 * channel. Anything unrecognized is passed through rather than guessed at —
 * mislabelling an unknown failure as "you declined" would tell the user a
 * falsehood about their own action.
 */
export function mapDeviceActionError(error) {
  const tag = error?._tag ?? error?.name ?? "";
  const message = String(error?.message ?? error?.originalError?.message ?? "");

  // Ledger's Ethereum app returns 0x6985 when the human rejects on-device.
  if (
    tag === "UserRejectedError" ||
    /0x6985|denied by the user|rejected/i.test(message)
  ) {
    return new DeviceRejectedError();
  }
  if (
    tag === "DeviceDisconnectedError" ||
    tag === "DeviceNotRecognizedError" ||
    /disconnect|not connected|no accessible device/i.test(message)
  ) {
    return new DeviceDisconnectedError();
  }
  return error instanceof Error ? error : new Error(message || "device action failed");
}

/** Build the context module with every remote endpoint pinned. */
function buildContextModule({ ContextModuleBuilder, intentId, originToken }) {
  return new ContextModuleBuilder({ originToken })
    // Descriptors come from our same-origin proxy, never Ledger's CAL directly.
    .setCalConfig({ url: descriptorProxyUrl(intentId), mode: "prod", branch: "main" })
    // Disabled outright — see the egress table above.
    .setWeb3ChecksConfig({ url: DISABLED_ENDPOINT })
    .setMetadataServiceConfig({ url: DISABLED_ENDPOINT })
    .setReporterConfig({ url: DISABLED_ENDPOINT })
    .build();
}

/**
 * Build a real device port over DMK.
 *
 * Returns `null` when WebHID is unavailable, matching the contract in
 * `device-adapter.ts`: a browser that cannot reach a device is an ordinary
 * outcome to render, not an exception to catch.
 */
export async function createDmkDevicePort({
  intentId,
  originToken = "ironclaw",
  expectedApp = "Ethereum",
} = {}) {
  if (!globalThis.navigator?.hid) return null;

  // Dynamically imported so DMK's weight stays in the review route's chunk and
  // never loads for ordinary chat users.
  const [{ DeviceManagementKitBuilder }, { webHidTransportFactory }, { SignerEthBuilder }, ctxModule] =
    await Promise.all([
      import("@ledgerhq/device-management-kit"),
      import("@ledgerhq/device-transport-kit-web-hid"),
      import("@ledgerhq/device-signer-kit-ethereum"),
      import("@ledgerhq/context-module"),
    ]);

  const dmk = new DeviceManagementKitBuilder().addTransport(webHidTransportFactory).build();
  let sessionId = null;

  return {
    async connect() {
      // Requires a user gesture; the caller only invokes this from a click.
      const device = await new Promise((resolve, reject) => {
        const subscription = dmk.startDiscovering({}).subscribe({
          next: (discovered) => {
            subscription.unsubscribe();
            resolve(discovered);
          },
          error: (error) => reject(mapDeviceActionError(error)),
        });
      });
      sessionId = await dmk.connect({ device });
      return { connected: true };
    },

    async status() {
      if (!sessionId) return DEVICE_STATUS.disconnected;
      try {
        const state = await new Promise((resolve, reject) => {
          const subscription = dmk.getDeviceSessionState({ sessionId }).subscribe({
            next: (value) => {
              subscription.unsubscribe();
              resolve(value);
            },
            error: reject,
          });
        });
        return mapSessionState(state, expectedApp);
      } catch {
        return DEVICE_STATUS.disconnected;
      }
    },

    async signTransaction({ unsignedPayload, derivationPath, clearSigningContext }) {
      if (!sessionId) throw new DeviceDisconnectedError();

      const signer = new SignerEthBuilder({ dmk, sessionId, originToken })
        .withContextModule(
          buildContextModule({
            ContextModuleBuilder: ctxModule.ContextModuleBuilder,
            intentId,
            originToken,
          }),
        )
        .build();

      // The server's bytes, untouched. Nothing in this file constructs or
      // re-encodes a transaction.
      const { observable } = signer.signTransaction(derivationPath, unsignedPayload, {
        clearSigningContext,
      });

      const signature = await new Promise((resolve, reject) => {
        const subscription = observable.subscribe({
          next: (state) => {
            if (state.status === "completed") {
              subscription.unsubscribe();
              resolve(state.output);
            } else if (state.status === "error") {
              subscription.unsubscribe();
              reject(mapDeviceActionError(state.error));
            }
          },
          error: (error) => reject(mapDeviceActionError(error)),
        });
      });

      return {
        signature,
        deviceMetadata: { transport: "web-hid" },
      };
    },

    async disconnect() {
      if (!sessionId) return;
      try {
        await dmk.disconnect({ sessionId });
      } finally {
        sessionId = null;
      }
    },
  };
}
