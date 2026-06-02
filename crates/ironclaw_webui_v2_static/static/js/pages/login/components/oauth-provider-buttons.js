import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";

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

// Renders only the OAuth redirect-button grid. The "or continue with"
// divider and outer section spacing are owned by the login page so the
// same divider covers OAuth and the NEAR wallet button together (or
// either one alone) without duplicating.
export function OAuthProviderButtons({ providers, redirectAfter }) {
  const t = useT();

  if (!providers.length) return null;

  return html`
    <div className="grid gap-2">
      ${providers.map(
        (provider) => html`
          <${Button}
            key=${provider}
            as="a"
            href=${oauthHref(provider, redirectAfter)}
            variant="secondary"
            fullWidth
            className="gap-2"
          >
            <${Icon} name="shield" className="h-4 w-4" />
            ${t("login.oauthProvider", {
              provider: OAUTH_PROVIDER_LABELS[provider] || provider,
            })}
          <//>
        `
      )}
    </div>
  `;
}
