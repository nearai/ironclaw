import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { useT } from "../../../lib/i18n";

const OAUTH_PROVIDER_LABELS = {
  google: "Google",
  github: "GitHub",
  apple: "Apple",
};

function oauthHref(provider, redirectAfter) {
  return `/auth/login/${encodeURIComponent(provider)}?redirect_after=${encodeURIComponent(
    redirectAfter
  )}`;
}

export function OAuthProviderButtons({ providers, redirectAfter }) {
  const t = useT();

  if (!providers.length) return null;

  return (
    <div className="mt-6 space-y-3">
      <div className="flex items-center gap-3 text-[11px] uppercase text-[var(--v2-text-faint)]">
        <span className="h-px flex-1 bg-[var(--v2-panel-border)]"></span>
        <span>{t("login.oauthDivider")}</span>
        <span className="h-px flex-1 bg-[var(--v2-panel-border)]"></span>
      </div>
      <div className="grid gap-2">
        {providers.map(
          (provider) => (
            <Button
              key={provider}
              as="a"
              href={oauthHref(provider, redirectAfter)}
              variant="secondary"
              fullWidth
              className="gap-2"
            >
              <Icon name="shield" className="h-4 w-4" />
              {t("login.oauthProvider", {
                provider: OAUTH_PROVIDER_LABELS[provider] || provider,
              })}
            </Button>
          )
        )}
      </div>
    </div>
  );
}
