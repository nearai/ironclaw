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
 * Security invariants (issue #4112):
 * - No raw token, PKCE verifier, opaque state, or auth code is ever handled
 *   by this component. The server supplies only an opaque IDP URL.
 * - window.open is called with `noopener,noreferrer` to prevent the popup
 *   from accessing this window's context.
 * - The URL is parsed and must have the "https:" protocol before opening to
 *   reject non-HTTPS schemes (javascript:, data:, custom protocol handlers).
 */
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { AuthGateShell } from "./auth-gate-shell.js";

export function AuthOauthCard({ gate, onCancel }) {
  const t = useT();
  const [opened, setOpened] = React.useState(false);
  const [error, setError] = React.useState("");
  const hasHttpsAuthorizationUrl = React.useMemo(() => {
    if (!gate.authorizationUrl) return false;
    try {
      return new URL(gate.authorizationUrl).protocol === "https:";
    } catch {
      return false;
    }
  }, [gate.authorizationUrl]);
  React.useEffect(() => {
    setError("");
  }, [gate.authorizationUrl, gate.gateRef, gate.runId]);

  const providerLabel = gate.provider
    ? gate.provider.charAt(0).toUpperCase() + gate.provider.slice(1)
    : t("authGate.oauthProviderFallback");

  const openAuth = React.useCallback(() => {
    // Guard: reject missing or non-HTTPS URLs before window.open so that
    // custom protocol handlers (javascript:, tel:, ms-msdt:, slack:) are
    // never opened even if a future code path writes an unexpected scheme.
    if (!hasHttpsAuthorizationUrl) {
      setError(t("authGate.serviceUnavailable"));
      return;
    }
    // Must be called synchronously in a click handler to be treated as a
    // user-gesture popup by the browser (not blocked by popup blockers).
    setError("");
    window.open(gate.authorizationUrl, "_blank", "noopener,noreferrer");
    setOpened(true);
  }, [gate.authorizationUrl, hasHttpsAuthorizationUrl]);

  const openLabel = opened
    ? t("authGate.reopenAuthorization", { provider: providerLabel })
    : t("authGate.openAuthorization", { provider: providerLabel });

  return html`
    <${AuthGateShell}
      icon="link"
      headline=${gate?.headline || t("authGate.oauthTitle")}
      provider=${gate?.provider ? providerLabel : ""}
      accountLabel=${gate?.accountLabel || ""}
      body=${gate?.body || ""}
      expiresAt=${gate?.expiresAt || ""}
      pillHint=${t("authGate.pillAuthorize")}
    >
      <div className="flex flex-wrap gap-2">
        <${Button}
          as="a"
          href=${hasHttpsAuthorizationUrl ? gate.authorizationUrl : undefined}
          target="_blank"
          rel="noopener noreferrer"
          className="auth-oauth"
          variant="primary"
          onClick=${(event) => {
            event.preventDefault();
            openAuth();
          }}
        >
          <${Icon} name="link" className="h-4 w-4" />
          ${openLabel}
        <//>
        <${Button}
          type="button"
          variant="secondary"
          onClick=${() => onCancel?.()}
        >
          ${t("authGate.cancel")}
        <//>
      </div>

      ${error &&
      html`
        <div
          className="mt-3 rounded-md border border-red-400/20 bg-red-500/10 px-3 py-2 text-xs text-red-200"
          role="alert"
        >
          ${error}
        </div>
      `}
      ${opened &&
      html`
        <p className="mt-2 text-xs text-iron-300">${t("authGate.oauthWaiting")}</p>
      `}
    <//>
  `;
}
