import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { useNearLogin } from "../hooks/useNearLogin.js";

// Per-status button label keys. NEAR is a challenge/verify flow with
// several round trips (wallet connect, message sign, server verify),
// so the button reflects progress instead of a single spinner.
const STATUS_LABEL_KEYS = {
  idle: "login.nearButton",
  connecting: "login.nearConnecting",
  signing: "login.nearSigning",
  verifying: "login.nearVerifying",
};

/**
 * NEAR wallet sign-in button. Unlike the OAuth providers (which are
 * plain `<a href>` redirects to `/auth/login/{provider}`), NEAR runs a
 * client-side NEP-413 challenge/verify handshake and then calls
 * `onToken` with the minted bearer — wired by the login page to
 * `auth.signIn`.
 */
export function NearLoginButton({ onToken }) {
  const t = useT();
  const { status, error, signInWithNear } = useNearLogin({ onToken });
  const busy = status !== "idle";
  const label = t(STATUS_LABEL_KEYS[status] || "login.nearButton");

  return html`
    <div className="grid gap-2">
      <${Button}
        type="button"
        variant="secondary"
        fullWidth
        className="gap-2"
        disabled=${busy}
        onClick=${signInWithNear}
      >
        <${Icon} name="plug" className="h-4 w-4" />
        ${label}
      <//>
      ${error &&
        html`<p
          className="text-xs text-[var(--v2-danger-text)]"
          role="alert"
        >${error}</p>`}
    </div>
  `;
}
