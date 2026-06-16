import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";

// Shared status surface for the NEAR AI / Codex login flows driven by
// `useProviderLogin`. Renders the Codex device code (when issued) plus the
// waiting / error messages for both providers. Both the onboarding screen and
// the Settings → Inference tab drop this in so the two surfaces stay identical.
export function ProviderLoginStatus({ login }) {
  const t = useT();
  const { nearaiBusy, nearaiError, codexBusy, codexError, codexCode, cancelNearai } = login;

  return html`
    ${nearaiBusy &&
    html`<div className="flex flex-col items-center gap-2 text-center text-xs text-[var(--v2-text-muted)]">
      <span>${t("onboarding.nearaiWaiting")}</span>
      ${typeof cancelNearai === "function" &&
      html`<button
        type="button"
        onClick=${() => cancelNearai()}
        className="inline-flex min-h-[36px] items-center rounded-[8px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 text-xs font-medium text-[var(--v2-text-strong)] hover:border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))]"
      >
        ${t("common.cancel")}
      </button>`}
    </div>`}
    ${nearaiError &&
    html`<div className="text-center text-xs text-[var(--v2-danger-text)]">${nearaiError}</div>`}

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
    html`<div className="text-center text-xs text-[var(--v2-danger-text)]">${codexError}</div>`}
  `;
}
