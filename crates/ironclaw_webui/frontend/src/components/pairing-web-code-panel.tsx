// @ts-nocheck
import { useQueryClient } from "@tanstack/react-query";
import QRCode from "qrcode";
import React from "react";
import { Button } from "../design-system/button";
import { useT } from "../lib/i18n";
import { notifyChannelConnected } from "../lib/channel-connection-events";
import {
  extensionPairingError,
  getExtensionPairingStatus,
  mintExtensionPairingCode,
  unpairExtension,
} from "../lib/extension-pairing-api";

const POLL_INTERVAL_MS = 2000;
const COUNTDOWN_INTERVAL_MS = 1000;
const COPIED_RESET_MS = 1500;

// "m:ss" until expiry, clamped at 0:00.
export function formatPairingCountdown(remainingMs) {
  const totalSeconds = Math.max(0, Math.ceil(remainingMs / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

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
// extension with the `web_generated_code` connect strategy: mint a code,
// render it as copyable text (+ deep link and QR when the backend supplies
// one), count down to expiry, and poll until the backend reports the account
// connected. Vendor copy rides the backend connection requirement
// (`instructions`); the panel itself is vendor-blind.
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
  const [qrDataUrl, setQrDataUrl] = React.useState("");
  const [now, setNow] = React.useState(() => Date.now());
  const [error, setError] = React.useState("");
  const [copiedTarget, setCopiedTarget] = React.useState("");
  const [isRenewing, setIsRenewing] = React.useState(false);
  const [isDisconnecting, setIsDisconnecting] = React.useState(false);
  // Only a connection observed to *happen* (a not-connected state seen first)
  // broadcasts + invalidates; mounting over an already-paired account is not a
  // connection event and must not re-trigger parked-thread resumes.
  const sawDisconnectedRef = React.useRef(false);
  const notifiedRef = React.useRef(false);
  const copiedTimerRef = React.useRef(null);
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
    setNow(Date.now());
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

  // Render the deep link as a QR data URL; a rotated code re-renders it.
  const deepLink = pending?.deep_link || "";
  React.useEffect(() => {
    if (!deepLink) {
      setQrDataUrl("");
      return undefined;
    }
    let cancelled = false;
    Promise.resolve(QRCode.toDataURL(deepLink))
      .then((dataUrl) => {
        if (!cancelled) setQrDataUrl(dataUrl);
      })
      .catch(() => {
        // The code + deep link remain usable without the QR.
        if (!cancelled) setQrDataUrl("");
      });
    return () => {
      cancelled = true;
    };
  }, [deepLink]);

  const expiresAtMs = pending ? pendingExpiresAtMs(pending) : 0;
  const expired = Boolean(pending) && now >= expiresAtMs;

  // Countdown tick while a live code is on screen.
  React.useEffect(() => {
    if (!pending || connected || expired) return undefined;
    const timer = setInterval(() => setNow(Date.now()), COUNTDOWN_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [pending, connected, expired]);

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

  React.useEffect(() => () => clearTimeout(copiedTimerRef.current), []);

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
      setQrDataUrl("");
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

  const copyText = async (target, text) => {
    const clipboard = typeof navigator === "undefined" ? null : navigator.clipboard;
    if (!clipboard?.writeText) return;
    try {
      await clipboard.writeText(text);
      setCopiedTarget(target);
      clearTimeout(copiedTimerRef.current);
      copiedTimerRef.current = setTimeout(() => setCopiedTarget(""), COPIED_RESET_MS);
    } catch (_) {
      // Clipboard can be blocked; the code stays visible for manual copy.
    }
  };

  const containerClass = compact
    ? "mt-3"
    : "mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4";

  if (connected) {
    return (
      <div data-testid="pairing-web-code-panel" className={containerClass}>
        <p data-testid="pairing-connected" className="text-sm text-[var(--v2-positive-text)]">
          ✅ {t("pairing.web.paired", { name: displayName || extensionId })}
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

  if (expired) {
    return (
      <div data-testid="pairing-web-code-panel" className={containerClass}>
        {!compact &&
        (<h4 className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
          {t("pairing.web.title", { name: displayName || extensionId })}
        </h4>)}
        <p data-testid="pairing-expired" className="text-xs leading-5 text-iron-300">
          {t("pairing.web.expired")}
        </p>
        <Button
          variant="secondary"
          size="sm"
          className="mt-2"
          onClick={renew}
          loading={isRenewing}
          data-testid="pairing-new-code"
        >
          {t("pairing.web.getNewCode")}
        </Button>
        {error &&
        (<p role="alert" className="mt-2 text-xs leading-5 text-red-300">{error}</p>)}
      </div>
    );
  }

  return (
    <div data-testid="pairing-web-code-panel" className={containerClass}>
      {!compact &&
      (<h4 className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
        {t("pairing.web.title", { name: displayName || extensionId })}
      </h4>)}
      <p className="mb-3 text-xs leading-5 text-iron-300">{instructions ||
        (deepLink
          ? t("pairing.web.instructions", { name: displayName || extensionId })
          : t("pairing.web.instructionsNoLink", { name: displayName || extensionId }))}</p>

      <div className="flex flex-col gap-3 sm:flex-row sm:items-start">
        {qrDataUrl &&
        (
          <img
            src={qrDataUrl}
            alt={t("pairing.web.qrAlt", { name: displayName || extensionId })}
            className="h-36 w-36 shrink-0 rounded-md border border-white/[0.06] bg-white p-1"
          />
        )}
        <div className="min-w-0 flex-1 space-y-2">
          <div className="flex flex-wrap items-center gap-2">
            <span
              data-testid="pairing-code"
              className="font-mono text-xl tracking-[0.18em] text-iron-100"
            >
              {pending.code}
            </span>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => copyText("code", pending.code)}
              data-testid="pairing-copy-code"
            >
              {copiedTarget === "code" ? t("common.copiedToClipboard") : t("pairing.web.copyCode")}
            </Button>
          </div>
          {deepLink &&
          (<div>
            <Button
              as="a"
              href={deepLink}
              target="_blank"
              rel="noreferrer"
              variant="secondary"
              size="sm"
              data-testid="pairing-open-link"
            >
              {t("pairing.web.openIn", { name: displayName || extensionId })}
            </Button>
          </div>)}
          <p data-testid="pairing-countdown" className="text-[11px] text-iron-400">
            {t("pairing.web.expiresIn", {
              time: formatPairingCountdown(expiresAtMs - now),
            })}
          </p>
        </div>
      </div>
      {error &&
      (<p role="alert" className="mt-3 text-xs leading-5 text-red-300">{error}</p>)}
    </div>
  );
}
