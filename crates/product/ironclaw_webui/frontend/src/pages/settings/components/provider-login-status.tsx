import { useT } from "../../../lib/i18n";

// Shared status surface for the NEAR AI / Codex login flows driven by
// `useProviderLogin`. Renders the Codex device code (when issued) plus the
// waiting / error messages for both providers. Both the onboarding screen and
// the Settings → Inference tab drop this in so the two surfaces stay identical.
export function ProviderLoginStatus({ login }) {
  const t = useT();
  const { nearaiBusy, nearaiError, codexBusy, codexError, codexCode } = login;

  return (
    <>
    {nearaiBusy &&
    (<div className="text-center text-ui-sm text-[var(--v2-text-muted)]">
      {t("onboarding.nearaiWaiting")}
    </div>)}
    {nearaiError &&
    (<div className="text-center text-ui-sm text-[var(--v2-danger-text)]">{nearaiError}</div>)}

    {codexCode &&
    (<div
      className="mx-auto max-w-md rounded-lg border border-[var(--v2-border)] bg-[var(--v2-surface-raised)] p-4 text-center"
    >
      <div className="text-ui-sm text-[var(--v2-text-muted)]">
        {t("onboarding.codexEnterCode")}
      </div>
      <div className="mt-2 font-mono text-title-lg font-semibold tracking-[0.3em] text-[var(--v2-text-strong)]">
        {codexCode.userCode}
      </div>
      <a
        className="mt-2 inline-block text-ui-sm underline hover:text-[var(--v2-text-strong)]"
        href={codexCode.verificationUri}
        target="_blank"
        rel="noopener noreferrer"
      >
        {codexCode.verificationUri}
      </a>
    </div>)}
    {codexBusy &&
    (<div className="text-center text-ui-sm text-[var(--v2-text-muted)]">
      {t("onboarding.codexWaiting")}
    </div>)}
    {codexError &&
    (<div className="text-center text-ui-sm text-[var(--v2-danger-text)]">{codexError}</div>)}
    </>
  );
}
