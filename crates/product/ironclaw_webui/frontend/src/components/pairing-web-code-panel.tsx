// @ts-nocheck
import { useQueryClient } from "@tanstack/react-query";
import React from "react";
import { Button } from "../design-system/button";
import { LinkPayloadPanel } from "./link-payload-panel";
import { useT } from "../lib/i18n";
import { notifyChannelConnected } from "../lib/channel-connection-events";
import {
  extensionPairingError,
  getExtensionPairingStatus,
  mintExtensionPairingCode,
  unpairExtension,
} from "../lib/extension-pairing-api";

const POLL_INTERVAL_MS = 2000;

function pendingExpiresAtMs(pending) {
  const parsed = Date.parse(pending?.expires_at || "");
  return Number.isFinite(parsed) ? parsed : 0;
}

function pendingIsLive(pending) {
  // The deep link is presentation sugar (absent until the channel's config
  // fills the template); a live CODE alone is fully pairable.
  return Boolean(pending?.code) && pendingExpiresAtMs(pending) > Date.now();
}

// In-chat (`compact`) and Extensions-page pairing panel for any channel
// extension with the `web_generated_code` connect strategy: mint a code, poll
// until the backend reports the account connected, and hand the code, deep
// link, and expiry to the shared `LinkPayloadPanel` for presentation. Vendor
// copy rides the backend connection requirement (`instructions`); the panel
// itself is vendor-blind.
//
// The QR, the countdown, the copy affordance, and the renewal button live in
// `LinkPayloadPanel` and are shared with the device-link card — this component
// owns only the pairing lifecycle (mint, poll, unpair).
export function PairingWebCodePanel({
  extensionId,
  displayName = "",
  instructions = "",
  compact = false,
}) {
  const t = useT();
  const queryClient = useQueryClient();
  const [connected, setConnected] = React.useState(false);
  const [pending, setPending] = React.useState(null);
  const [error, setError] = React.useState("");
  const [isRenewing, setIsRenewing] = React.useState(false);
  const [isDisconnecting, setIsDisconnecting] = React.useState(false);
  // Only a connection observed to *happen* (a not-connected state seen first)
  // broadcasts + invalidates; mounting over an already-paired account is not a
  // connection event and must not re-trigger parked-thread resumes.
  const sawDisconnectedRef = React.useRef(false);
  const notifiedRef = React.useRef(false);
  // Every disconnect advances this epoch before issuing DELETE. Polls capture
  // the epoch they started in and may not publish a result from an older one.
  const pairingEpochRef = React.useRef(0);

  const markConnected = () => {
    setConnected(true);
    if (!sawDisconnectedRef.current || notifiedRef.current) return;
    notifiedRef.current = true;
    notifyChannelConnected({ channel: extensionId, source: "pairing-web-code-panel" });
    queryClient.invalidateQueries({ queryKey: ["extensions"] });
    queryClient.invalidateQueries({ queryKey: ["connectable-channels"] });
  };

  const adoptPending = (next) => {
    setPending((current) =>
      current &&
      next &&
      current.code === next.code &&
      current.expires_at === next.expires_at
        ? current
        : next,
    );
  };

  const mintCode = async () => {
    const minted = await mintExtensionPairingCode(extensionId);
    adoptPending(minted);
  };

  // Mount: reuse an unexpired pending code when the backend still has one,
  // otherwise mint a fresh one; skip both when already connected.
  React.useEffect(() => {
    let cancelled = false;
    const bootstrap = async () => {
      try {
        const status = await getExtensionPairingStatus(extensionId);
        if (cancelled) return;
        if (status?.connected) {
          setConnected(true);
          return;
        }
        sawDisconnectedRef.current = true;
        if (pendingIsLive(status?.pending)) {
          adoptPending(status.pending);
          return;
        }
        await mintCode();
      } catch (bootstrapError) {
        if (!cancelled) {
          setError(extensionPairingError(bootstrapError, t("pairing.web.loadFailed", { name: displayName || extensionId })));
        }
      }
    };
    bootstrap();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [extensionId]);

  // Poll the pairing status until connected; also adopt a code another
  // tab/session rotated so both surfaces show the same live code.
  React.useEffect(() => {
    if (connected) return undefined;
    const timer = setInterval(async () => {
      const pairingEpoch = pairingEpochRef.current;
      try {
        const status = await getExtensionPairingStatus(extensionId);
        if (pairingEpoch !== pairingEpochRef.current) return;
        if (status?.connected) {
          markConnected();
          return;
        }
        sawDisconnectedRef.current = true;
        if (pendingIsLive(status?.pending)) {
          adoptPending(status.pending);
        }
      } catch (_) {
        // Poll is best-effort; the next tick retries.
      }
    }, POLL_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [connected, extensionId]);

  const renew = async () => {
    if (isRenewing) return;
    setError("");
    setIsRenewing(true);
    try {
      await mintCode();
    } catch (renewError) {
      setError(extensionPairingError(renewError, t("pairing.web.loadFailed", { name: displayName || extensionId })));
    } finally {
      setIsRenewing(false);
    }
  };

  const disconnect = async () => {
    if (isDisconnecting) return;
    setError("");
    setIsDisconnecting(true);
    // Invalidate every poll that started before this disconnect. Its response
    // describes the old pairing and must not reconnect the local UI.
    pairingEpochRef.current += 1;
    try {
      await unpairExtension(extensionId);
      notifiedRef.current = false;
      sawDisconnectedRef.current = true;
      setConnected(false);
      setPending(null);
      queryClient.invalidateQueries({ queryKey: ["extensions"] });
      queryClient.invalidateQueries({ queryKey: ["connectable-channels"] });
    } catch (disconnectError) {
      setError(extensionPairingError(disconnectError, t("pairing.web.disconnectFailed", { name: displayName || extensionId })));
      setIsDisconnecting(false);
      return;
    }
    try {
      // The disconnect already succeeded; failing to mint the NEXT pairing
      // code is a load problem, never a failed disconnect.
      await mintCode();
    } catch (mintError) {
      setError(extensionPairingError(mintError, t("pairing.web.loadFailed", { name: displayName || extensionId })));
    } finally {
      setIsDisconnecting(false);
    }
  };

  const containerClass = compact
    ? "mt-3"
    : "mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4";
  const name = displayName || extensionId;

  if (connected) {
    return (
      <div data-testid="pairing-web-code-panel" className={containerClass}>
        <p data-testid="pairing-connected" className="text-sm text-[var(--v2-positive-text)]">
          ✅ {t("pairing.web.paired", { name })}
        </p>
        <button
          type="button"
          onClick={disconnect}
          disabled={isDisconnecting}
          data-testid="pairing-disconnect"
          className="mt-2 text-xs text-iron-400 underline underline-offset-2 hover:text-iron-200 disabled:cursor-not-allowed disabled:opacity-60"
        >
          {t("pairing.web.disconnect")}
        </button>
        {error &&
        (<p role="alert" className="mt-2 text-xs leading-5 text-red-300">{error}</p>)}
      </div>
    );
  }

  if (!pending) {
    return (
      <div data-testid="pairing-web-code-panel" className={containerClass}>
        {error
          ? (
              <div className="space-y-2">
                <p role="alert" className="text-xs leading-5 text-red-300">{error}</p>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={renew}
                  loading={isRenewing}
                  data-testid="pairing-new-code"
                >
                  {t("pairing.web.getNewCode")}
                </Button>
              </div>
            )
          : (<div className="v2-skeleton h-3 w-24 rounded" />)}
      </div>
    );
  }

  const deepLink = pending.deep_link || "";
  return (
    <div data-testid="pairing-web-code-panel" className={containerClass}>
      {!compact &&
      (<h4 className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
        {t("pairing.web.title", { name })}
      </h4>)}
      <p className="mb-3 text-xs leading-5 text-iron-300">{instructions ||
        (deepLink
          ? t("pairing.web.instructions", { name })
          : t("pairing.web.instructionsNoLink", { name }))}</p>

      <LinkPayloadPanel
        idPrefix="pairing"
        payload={deepLink}
        code={pending.code}
        expiresAtMs={pendingExpiresAtMs(pending)}
        labels={pairingLabels(t, name)}
        onRenew={renew}
        isRenewing={isRenewing}
      />
      {error &&
      (<p role="alert" className="mt-3 text-xs leading-5 text-red-300">{error}</p>)}
    </div>
  );
}

function pairingLabels(t, name) {
  return {
    qrAlt: t("pairing.web.qrAlt", { name }),
    copy: t("pairing.web.copyCode"),
    copied: t("common.copiedToClipboard"),
    open: t("pairing.web.openIn", { name }),
    expiresIn: (time) => t("pairing.web.expiresIn", { time }),
    expired: t("pairing.web.expired"),
    renew: t("pairing.web.getNewCode"),
  };
}
