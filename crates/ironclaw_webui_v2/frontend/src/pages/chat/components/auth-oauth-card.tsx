/**
 * AuthOauthCard — rendered when `gate.challengeKind === "oauth_url"`.
 *
 * Status Pill + Drawer presentation (AuthGateShell). The drawer holds the
 * authorization CTA and waiting/expiry metadata.
 *
 * Opens `gate.authorizationUrl` in a new browser tab via a user-gesture
 * click. The OAuth callback is handled server-side
 * (`/api/reborn/product-auth/oauth/callback/{flow_id}`), which resumes the
 * paused run. The callback page emits a same-origin completion signal so the
 * WebUI can clear this gate immediately, then projection_update confirms the
 * resumed run state.
 *
 * While the popup is open we show a spinner ("waiting to authorize"); if the
 * popup is closed before the gate clears, we surface a "closed before
 * finishing" notice with a re-open CTA. Completion still arrives via the
 * completion signal or the resumed-run projection — this is UI feedback only.
 *
 * Security invariants (issue #4112):
 * - No raw token, PKCE verifier, opaque state, or auth code is ever handled
 *   by this component. The server supplies only an opaque IDP URL.
 * - window.open is called with `noopener,noreferrer` to prevent the popup
 *   from accessing this window's context.
 * - The URL is parsed and must have the "https:" protocol before opening to
 *   reject non-HTTPS schemes (javascript:, data:, custom protocol handlers).
 */
import React from "react";
import { useT } from "../../../lib/i18n";
import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { openAuthPopup } from "../../../lib/product-auth-oauth-events";
import { AuthGateShell } from "./auth-gate-shell";

// After a popup closes we wait briefly before flagging it as abandoned: a
// successful callback closes the popup itself, and the gate then clears via
// the completion signal / resumed-run projection. This grace window avoids a
// "closed before finishing" flash on a normal success.
const CLOSED_NOTICE_GRACE_MS = 1500;

// User-facing names for OAuth provider ids. The gate payload carries the raw
// provider id (e.g. `slack_personal`), which must never leak into copy —
// "Re-open Slack_personal authorization" is not a sentence.
const PROVIDER_DISPLAY_NAMES = {
  google: "Google",
  slack_personal: "Slack",
  github: "GitHub",
  notion: "Notion",
  nearai: "NEAR AI",
};

function providerDisplayName(providerId) {
  if (PROVIDER_DISPLAY_NAMES[providerId]) return PROVIDER_DISPLAY_NAMES[providerId];
  return providerId
    .split("_")
    .map((word) => (word ? word.charAt(0).toUpperCase() + word.slice(1) : word))
    .join(" ");
}

export function AuthOauthCard({ gate, onCancel }) {
  const t = useT();
  const [opened, setOpened] = React.useState(false);
  const [error, setError] = React.useState("");
  const [closedNotice, setClosedNotice] = React.useState(false);
  // Bumped on every open so the close-watcher effect restarts for the new popup.
  const [watchNonce, setWatchNonce] = React.useState(0);
  const popupRef = React.useRef(null);
  const hasHttpsAuthorizationUrl = React.useMemo(() => {
    if (!gate.authorizationUrl) return false;
    try {
      return new URL(gate.authorizationUrl).protocol === "https:";
    } catch {
      return false;
    }
  }, [gate.authorizationUrl]);

  // Reset transient UI when the gate changes (new run / new flow).
  React.useEffect(() => {
    setError("");
    setOpened(false);
    setClosedNotice(false);
  }, [gate.authorizationUrl, gate.gateRef, gate.runId]);

  // While a popup is open, watch for it closing. On close, wait a grace period
  // (the callback closes the popup on success too) before surfacing the
  // "closed before finishing" notice.
  React.useEffect(() => {
    if (!watchNonce) return undefined;
    let graceTimer = null;
    const poll = window.setInterval(() => {
      const popup = popupRef.current;
      if (popup && popup.closed) {
        window.clearInterval(poll);
        graceTimer = window.setTimeout(() => setClosedNotice(true), CLOSED_NOTICE_GRACE_MS);
      }
    }, 500);
    return () => {
      window.clearInterval(poll);
      if (graceTimer) window.clearTimeout(graceTimer);
    };
  }, [watchNonce]);

  const providerLabel = gate.provider
    ? providerDisplayName(gate.provider)
    : t("authGate.oauthProviderFallback");

  const openAuth = React.useCallback(() => {
    // Guard: reject missing or non-HTTPS URLs before window.open so that
    // custom protocol handlers (javascript:, tel:, ms-msdt:, slack:) are
    // never opened even if a future code path writes an unexpected scheme.
    // openAuthPopup re-checks HTTPS before navigating.
    if (!hasHttpsAuthorizationUrl) {
      setError(t("authGate.serviceUnavailable"));
      return;
    }
    // Must be called synchronously in a click handler to be treated as a
    // user-gesture popup by the browser (not blocked by popup blockers).
    // The sized `about:blank` pre-open keeps authorization in a popup
    // (matching the onboarding and configure flows) and reliably detects a
    // blocked popup — the noopener fresh-open path returns null even on
    // success. The opener is severed before navigation; gate completion
    // travels via the localStorage/BroadcastChannel contract, which never
    // uses window.opener.
    setError("");
    setClosedNotice(false);
    const popup = window.open("about:blank", "_blank", "width=600,height=600");
    if (!popup) {
      setError(t("authGate.popupBlocked"));
      return;
    }
    popup.opener = null;
    popupRef.current = popup;
    openAuthPopup(gate.authorizationUrl, popup);
    setOpened(true);
    setWatchNonce((nonce) => nonce + 1);
  }, [gate.authorizationUrl, hasHttpsAuthorizationUrl, t]);

  const openLabel = opened
    ? t("authGate.reopenAuthorization", { provider: providerLabel })
    : t("authGate.openAuthorization", { provider: providerLabel });

  // Popup open and the gate hasn't cleared yet — show the waiting spinner.
  const awaiting = opened && !closedNotice;

  return (
    <AuthGateShell
      icon="link"
      headline={gate?.headline || t("authGate.oauthTitle")}
      provider={gate?.provider ? providerLabel : ""}
      accountLabel={gate?.accountLabel || ""}
      body={gate?.body || ""}
      expiresAt={gate?.expiresAt || ""}
      pillHint={t("authGate.pillAuthorize")}
      challengeKind="oauth_url"
    >
      <div className="flex flex-wrap gap-2">
        <Button
          as="a"
          href={hasHttpsAuthorizationUrl ? gate.authorizationUrl : undefined}
          target="_blank"
          rel="noopener noreferrer"
          className="auth-oauth"
          data-testid="auth-oauth-open"
          variant="primary"
          loading={awaiting}
          onClick={(event) => {
            event.preventDefault();
            openAuth();
          }}
        >
          {!awaiting && <Icon name="link" className="h-4 w-4" />}
          {awaiting ? t("authGate.authorizing", { provider: providerLabel }) : openLabel}
        </Button>
        <Button
          type="button"
          variant="secondary"
          onClick={() => onCancel?.()}
        >
          {t("authGate.cancel")}
        </Button>
      </div>

      {error &&
      (
        <div
          className="mt-3 rounded-md border border-red-400/20 bg-red-500/10 px-3 py-2 text-xs text-red-200"
          role="alert"
        >
          {error}
        </div>
      )}
      {closedNotice &&
      (
        <div
          className="mt-3 rounded-md border border-amber-400/25 bg-amber-500/10 px-3 py-2 text-xs text-amber-200"
          role="status"
        >
          The {providerLabel} authorization window closed before you finished
          connecting. Re-open it above to try again.
        </div>
      )}
    </AuthGateShell>
  );
}
