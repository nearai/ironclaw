// @ts-nocheck
/**
 * AuthDeviceLinkCard — rendered when `gate.challengeKind === "device_link"`.
 *
 * Status Pill + Drawer presentation (AuthGateShell), like every other auth
 * gate. The drawer holds the whole multi-step link: the scannable payload and
 * its countdown, the "waiting for the vendor" frame, the phone-number /
 * login-code / account-password inputs, and the success or failure ending.
 *
 * The flow itself lives in the shared `DeviceLinkPanel` because the Extensions
 * configure modal renders the same link from outside a run — this file is the
 * gate chrome and the gate → panel wiring, nothing more.
 *
 * Security notes:
 * - The scannable payload IS the vendor's login token: whoever renders it can
 *   invite a device onto the account. It is shown to the account's own user in
 *   their own session and never logged or forwarded.
 * - The login code and account password are handed to the host and dropped;
 *   the session they produce is host custody and never reaches the browser.
 */
import React from "react";
import { useT } from "../../../lib/i18n";
import { Button } from "../../../design-system/button";
import { DeviceLinkPanel } from "../../../components/device-link-panel";
import { AuthGateShell } from "./auth-gate-shell";

export function AuthDeviceLinkCard({ gate, onCancel }) {
  const t = useT();
  const frame = gate?.deviceLink || null;
  const displayName = frame?.displayName || gate?.accountLabel || gate?.provider || "";

  return (
    <AuthGateShell
      icon="link"
      headline={gate?.headline || t("deviceLink.title", { name: displayName })}
      provider={gate?.provider || ""}
      accountLabel={gate?.accountLabel || ""}
      body={gate?.body || ""}
      pillHint={t("deviceLink.pillLink")}
      challengeKind="device_link"
      testId="auth-device-link-card"
    >
      <DeviceLinkPanel
        provider={gate?.provider || ""}
        // The installed extension, not the credential-authority namespace.
        // These are two identities the repo keeps apart on purpose; passing
        // `provider` for both was the conflation that reads as correct right
        // up until one vendor backs more than one extension. The frame now
        // carries the installed id, so fall back to `provider` only for a
        // frame minted before it existed.
        extensionName={frame?.extensionId || gate?.provider || ""}
        displayName={displayName}
        initialFrame={frame}
        threadId={gate?.threadId || ""}
        runId={gate?.runId || ""}
        gateRef={gate?.gateRef || ""}
        invocationId={gate?.invocationId || ""}
      />
      <div className="mt-3">
        <Button type="button" variant="secondary" size="sm" onClick={() => onCancel?.()}>
          {t("authGate.cancel")}
        </Button>
      </div>
    </AuthGateShell>
  );
}
