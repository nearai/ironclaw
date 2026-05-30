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
 */
import { React, html } from "../../../lib/html.js";

export function AuthOauthCard({ gate, onCancel }) {
  const [opened, setOpened] = React.useState(false);

  const openAuth = React.useCallback(() => {
    if (!gate.authorizationUrl) return;
    // Must be called synchronously in a click handler to be treated as a
    // user-gesture popup by the browser (not blocked by popup blockers).
    window.open(gate.authorizationUrl, "_blank", "noopener,noreferrer");
    setOpened(true);
  }, [gate.authorizationUrl]);

  const providerLabel = gate.provider
    ? gate.provider.charAt(0).toUpperCase() + gate.provider.slice(1)
    : "the provider";

  return html`
    <div class="auth-oauth-card border rounded-xl p-4 bg-surface shadow-sm space-y-3">
      <div class="font-semibold text-base">
        ${gate.headline || \`Authorize \${providerLabel}\`}
      </div>

      ${gate.accountLabel && html`
        <div class="text-sm text-muted">
          Account: ${gate.accountLabel}
        </div>
      `}

      ${gate.body && html`
        <div class="text-sm">${gate.body}</div>
      `}

      <div class="flex gap-2 pt-1">
        <button
          type="button"
          class="btn btn-primary flex-1"
          onClick=${openAuth}
          disabled=${!gate.authorizationUrl}
        >
          ${opened
            ? \`Re-open \${providerLabel} authorization\`
            : \`Open \${providerLabel} authorization\`}
        </button>

        ${onCancel && html`
          <button
            type="button"
            class="btn btn-secondary"
            onClick=${onCancel}
          >
            Cancel
          </button>
        `}
      </div>

      ${opened && html`
        <div class="text-xs text-muted">
          Waiting for authorization to complete\u2026 You can close the popup tab
          once you\u2019ve approved access.
        </div>
      `}

      ${gate.expiresAt && html`
        <div class="text-xs text-muted">
          Expires: ${new Date(gate.expiresAt).toLocaleString()}
        </div>
      `}
    </div>
  `;
}
