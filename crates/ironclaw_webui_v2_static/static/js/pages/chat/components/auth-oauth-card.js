/**
 * AuthOauthCard — rendered when `gate.challengeKind === "oauth_url"`.
 *
 * Opens `gate.authorizationUrl` in a new browser tab via a user-gesture
 * click. The OAuth callback is handled server-side
 * (`/api/reborn/product-auth/oauth/callback/{flow_id}`), which resumes the
 * paused run. The WebUI observes the resume via the next projection_update
 * (run_status flip) which causes `setPendingGate(null)` via the `item.text`
 * path in useChatEvents.js — so this card unmounts automatically.
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

export function AuthOauthCard({ gate, onCancel }) {
  const t = useT();
  const [opened, setOpened] = React.useState(false);
  const hasHttpsAuthorizationUrl = React.useMemo(() => {
    if (!gate.authorizationUrl) return false;
    try {
      return new URL(gate.authorizationUrl).protocol === "https:";
    } catch {
      return false;
    }
  }, [gate.authorizationUrl]);

  const providerLabel = gate.provider
    ? gate.provider.charAt(0).toUpperCase() + gate.provider.slice(1)
    : t("authGate.oauthProviderFallback");

  const openAuth = React.useCallback(() => {
    // Guard: reject missing or non-HTTPS URLs before window.open so that
    // custom protocol handlers (javascript:, tel:, ms-msdt:, slack:) are
    // never opened even if a future code path writes an unexpected scheme.
    if (!hasHttpsAuthorizationUrl) return;
    // Must be called synchronously in a click handler to be treated as a
    // user-gesture popup by the browser (not blocked by popup blockers).
    window.open(gate.authorizationUrl, "_blank", "noopener,noreferrer");
    setOpened(true);
  }, [gate.authorizationUrl, hasHttpsAuthorizationUrl]);

  const openLabel = opened
    ? t("authGate.reopenAuthorization", { provider: providerLabel })
    : t("authGate.openAuthorization", { provider: providerLabel });

  return html`
    <form
      className="mx-auto w-full max-w-lg rounded-xl border border-[rgba(76,167,230,0.34)] bg-[rgba(76,167,230,0.08)] p-4"
      onSubmit=${(e) => e.preventDefault()}
    >
      <div className="mb-3 flex items-center gap-2">
        <span className="grid h-8 w-8 place-items-center rounded-md border border-[rgba(76,167,230,0.28)] bg-[rgba(76,167,230,0.1)] text-[#8fc8f2]">
          <${Icon} name="link" className="h-4 w-4" />
        </span>
        <span className="font-semibold text-white">
          ${gate?.headline || t("authGate.oauthTitle")}
        </span>
      </div>

      ${gate?.accountLabel && html`
        <div className="mb-2 text-xs text-iron-300">
          ${t("authGate.oauthAccountLabel")} ${gate.accountLabel}
        </div>
      `}

      ${gate?.body && html`
        <div className="mb-3 text-sm text-iron-200">${gate.body}</div>
      `}

      <div className="flex flex-wrap gap-2">
        <${Button}
          as="a"
          href=${hasHttpsAuthorizationUrl ? gate.authorizationUrl : undefined}
          target="_blank"
          rel="noopener noreferrer"
          className="auth-oauth"
          variant="primary"
          disabled=${!hasHttpsAuthorizationUrl}
          onClick=${(event) => {
            event.preventDefault();
            openAuth();
          }}
        >
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

      ${opened && html`
        <p className="mt-2 text-xs text-iron-300">
          ${t("authGate.oauthWaiting")}
        </p>
      `}

      ${gate?.expiresAt && html`
        <p className="mt-1 text-xs text-iron-300">
          ${t("authGate.expiresAt")}: ${new Date(gate.expiresAt).toLocaleString()}
        </p>
      `}
    </form>
  `;
}
