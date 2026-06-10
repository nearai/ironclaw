import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";

// NEAR AI's hosted auth (private.near.ai) rejects `frontend_callback` URLs that
// point at a loopback host, so its browser sign-in (GitHub / Google / NEAR
// Wallet) cannot complete from a local dev origin. Detect that origin so the
// caller surfaces an upfront notice instead of letting the user hit the opaque
// `Invalid frontend_callback` error in the opened tab (issue #4705).
function isLocalDevOrigin() {
  if (typeof window === "undefined" || !window.location) return false;
  const host = window.location.hostname;
  return (
    host === "localhost" ||
    host === "127.0.0.1" ||
    host === "0.0.0.0" ||
    host === "::1" ||
    host === "[::1]" ||
    host.endsWith(".localhost")
  );
}

// Shared status surface for the NEAR AI / Codex login flows driven by
// `useProviderLogin`. Renders the Codex device code (when issued) plus the
// waiting / error messages for both providers. Both the onboarding screen and
// the Settings → Inference tab drop this in so the two surfaces stay identical.
// `nearaiSsoAvailable` is set by the caller when the NEAR AI browser sign-in
// actions are actually on screen; combined with a local origin it gates the
// upfront "SSO unavailable on localhost" notice.
export function ProviderLoginStatus({ login, nearaiSsoAvailable = false }) {
  const t = useT();
  const { nearaiBusy, nearaiError, codexBusy, codexError, codexCode } = login;
  const showNearaiLocalNotice = nearaiSsoAvailable && isLocalDevOrigin();

  return html`
    ${showNearaiLocalNotice &&
    html`<div
      role="note"
      className="rounded-md border border-[var(--v2-warning-text)]/30 bg-[var(--v2-warning-text)]/10 px-3 py-2 text-center text-xs text-[var(--v2-warning-text)]"
    >
      ${t("onboarding.nearaiLocalSso")}
    </div>`}
    ${nearaiBusy &&
    html`<div className="text-center text-xs text-[var(--v2-text-muted)]">
      ${t("onboarding.nearaiWaiting")}
    </div>`}
    ${nearaiError &&
    html`<div className="text-center text-xs text-red-300">${nearaiError}</div>`}

    ${codexCode &&
    html`<div
      className="mx-auto max-w-md rounded-lg border border-[var(--v2-border)] bg-[var(--v2-surface-raised)] p-4 text-center"
    >
      <div className="text-xs text-[var(--v2-text-muted)]">
        ${t("onboarding.codexEnterCode")}
      </div>
      <div className="mt-2 font-mono text-2xl font-semibold tracking-[0.3em] text-[var(--v2-text-strong)]">
        ${codexCode.userCode}
      </div>
      <a
        className="mt-2 inline-block text-xs underline hover:text-[var(--v2-text-strong)]"
        href=${codexCode.verificationUri}
        target="_blank"
        rel="noopener noreferrer"
      >
        ${codexCode.verificationUri}
      </a>
    </div>`}
    ${codexBusy &&
    html`<div className="text-center text-xs text-[var(--v2-text-muted)]">
      ${t("onboarding.codexWaiting")}
    </div>`}
    ${codexError &&
    html`<div className="text-center text-xs text-red-300">${codexError}</div>`}
  `;
}
