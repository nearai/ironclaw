import { Text } from "@ironclaw/design-system";
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
    (<Text as="div" variant="caption" tone="muted" className="text-center">
      {t("onboarding.nearaiWaiting")}
    </Text>)}
    {nearaiError &&
    (<Text as="div" variant="caption" tone="danger" className="text-center">{nearaiError}</Text>)}

    {codexCode &&
    (<div
      className="mx-auto max-w-md rounded-lg border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] p-4 text-center"
    >
      <Text as="div" variant="caption" tone="muted">
        {t("onboarding.codexEnterCode")}
      </Text>
      <div className="mt-2 font-mono text-2xl font-medium tracking-[var(--v2-tracking-code)] text-[var(--v2-text-strong)]">
        {codexCode.userCode}
      </div>
      <a
        className="mt-2 inline-block text-xs underline hover:text-[var(--v2-text-strong)]"
        href={codexCode.verificationUri}
        target="_blank"
        rel="noopener noreferrer"
      >
        {codexCode.verificationUri}
      </a>
    </div>)}
    {codexBusy &&
    (<Text as="div" variant="caption" tone="muted" className="text-center">
      {t("onboarding.codexWaiting")}
    </Text>)}
    {codexError &&
    (<Text as="div" variant="caption" tone="danger" className="text-center">{codexError}</Text>)}
    </>
  );
}
