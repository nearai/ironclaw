// @ts-nocheck
import QRCode from "qrcode";
import React from "react";
import { Button } from "../design-system/button";

const COUNTDOWN_INTERVAL_MS = 1000;
const COPIED_RESET_MS = 1500;

// "m:ss" until expiry, clamped at 0:00.
export function formatLinkCountdown(remainingMs) {
  const totalSeconds = Math.max(0, Math.ceil(remainingMs / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

/**
 * LinkPayloadPanel — the ONE implementation of "here is a short-lived payload,
 * scan it or copy it before it expires".
 *
 * Both linking surfaces render through this component: the host-issued channel
 * pairing code (`pairing-web-code-panel`) and the vendor-issued device-link
 * payload (`device-link-panel`). Keeping a second QR/countdown/copy
 * implementation next to this one is exactly how the two drift.
 *
 * Presentation only. It owns no network calls, no polling, and no concept of
 * *what* the payload authorizes — it renders a string as a QR code, shows a
 * code, counts down, copies, and offers a renewal when the deadline passes.
 * The owner decides what "renew" does and when the panel is mounted at all.
 *
 * Props
 *   payload      openable/scannable content; drives the QR image and the
 *                "open" affordance. Empty renders neither.
 *   showQr       whether this payload is meant to be SCANNED. False renders the
 *                payload's other affordances without a QR image and never asks
 *                the encoder for one — a payload the owner knows is a link to
 *                open has nothing to scan.
 *   code         short code shown as text and offered to the clipboard.
 *   expiresAtMs  epoch ms the payload dies at; 0 disables the countdown and
 *                the expired view entirely.
 *   idPrefix     `data-testid` prefix, so a host surface keeps the test ids it
 *                already published (e2e selectors are a contract).
 *   labels       all user-visible copy, supplied by the owner:
 *                { qrAlt, copy, copied, open, expiresIn(time), expired, renew }
 *   onRenew      renewal handler; without one the expired view is copy only.
 *   isRenewing   spinner/disable state for the renewal button.
 *   onExpire     optional notification fired once per deadline crossing.
 */
export function LinkPayloadPanel({
  payload = "",
  showQr = true,
  code = "",
  expiresAtMs = 0,
  idPrefix = "link",
  labels = {},
  onRenew = null,
  isRenewing = false,
  onExpire = null,
}) {
  const [qrDataUrl, setQrDataUrl] = React.useState("");
  const [now, setNow] = React.useState(() => Date.now());
  const [copied, setCopied] = React.useState(false);
  const copiedTimerRef = React.useRef(null);
  // The deadline the `onExpire` notification has already fired for, so a
  // re-render (or a countdown tick past the deadline) cannot fire it twice
  // while a fresh payload still gets its own notification.
  const notifiedDeadlineRef = React.useRef(0);

  // Render the payload as a QR data URL; a rotated payload re-renders it.
  React.useEffect(() => {
    if (!payload || !showQr) {
      setQrDataUrl("");
      return undefined;
    }
    let cancelled = false;
    Promise.resolve(QRCode.toDataURL(payload))
      .then((dataUrl) => {
        if (!cancelled) setQrDataUrl(dataUrl);
      })
      .catch(() => {
        // The code and the open affordance remain usable without the QR.
        if (!cancelled) setQrDataUrl("");
      });
    return () => {
      cancelled = true;
    };
  }, [payload, showQr]);

  // A new deadline re-arms the clock: the countdown restarts and the expiry
  // notification is allowed to fire again.
  React.useEffect(() => {
    setNow(Date.now());
  }, [expiresAtMs]);

  const expired = expiresAtMs > 0 && now >= expiresAtMs;

  // Countdown tick while a live payload is on screen. Stops at expiry, so a
  // card parked on a dead payload holds no timer.
  React.useEffect(() => {
    if (!expiresAtMs || expired) return undefined;
    const timer = setInterval(() => setNow(Date.now()), COUNTDOWN_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [expiresAtMs, expired]);

  React.useEffect(() => {
    if (!expired || !onExpire) return;
    if (notifiedDeadlineRef.current === expiresAtMs) return;
    notifiedDeadlineRef.current = expiresAtMs;
    onExpire();
  }, [expired, expiresAtMs, onExpire]);

  React.useEffect(() => () => clearTimeout(copiedTimerRef.current), []);

  const copyCode = async () => {
    const clipboard = typeof navigator === "undefined" ? null : navigator.clipboard;
    if (!clipboard?.writeText || !code) return;
    try {
      await clipboard.writeText(code);
      setCopied(true);
      clearTimeout(copiedTimerRef.current);
      copiedTimerRef.current = setTimeout(() => setCopied(false), COPIED_RESET_MS);
    } catch (_) {
      // Clipboard can be blocked; the code stays visible for manual copy.
    }
  };

  if (expired) {
    return (
      <div data-testid={`${idPrefix}-payload-panel`}>
        <p data-testid={`${idPrefix}-expired`} className="text-xs leading-5 text-iron-300">
          {labels.expired}
        </p>
        {onRenew &&
        (
          <Button
            variant="secondary"
            size="sm"
            className="mt-2"
            onClick={onRenew}
            loading={isRenewing}
            data-testid={`${idPrefix}-new-code`}
          >
            {labels.renew}
          </Button>
        )}
      </div>
    );
  }

  return (
    <div
      data-testid={`${idPrefix}-payload-panel`}
      className="flex flex-col gap-3 sm:flex-row sm:items-start"
    >
      {qrDataUrl &&
      (
        <img
          src={qrDataUrl}
          alt={labels.qrAlt}
          className="h-36 w-36 shrink-0 rounded-md border border-white/[0.06] bg-white p-1"
        />
      )}
      <div className="min-w-0 flex-1 space-y-2">
        {code &&
        (
          <div className="flex flex-wrap items-center gap-2">
            <span
              data-testid={`${idPrefix}-code`}
              className="font-mono text-xl tracking-[0.18em] text-iron-100"
            >
              {code}
            </span>
            <Button
              variant="ghost"
              size="sm"
              onClick={copyCode}
              data-testid={`${idPrefix}-copy-code`}
            >
              {copied ? labels.copied : labels.copy}
            </Button>
          </div>
        )}
        {payload && labels.open &&
        (
          <div>
            <Button
              as="a"
              href={payload}
              target="_blank"
              rel="noreferrer"
              variant="secondary"
              size="sm"
              data-testid={`${idPrefix}-open-link`}
            >
              {labels.open}
            </Button>
          </div>
        )}
        {expiresAtMs > 0 &&
        (
          <p data-testid={`${idPrefix}-countdown`} className="text-[11px] text-iron-400">
            {labels.expiresIn?.(formatLinkCountdown(expiresAtMs - now))}
          </p>
        )}
      </div>
    </div>
  );
}
