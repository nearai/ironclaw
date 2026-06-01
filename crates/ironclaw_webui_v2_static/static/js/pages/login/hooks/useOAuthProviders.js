import { fetchAuthProviders } from "../../../lib/api.js";
import { React } from "../../../lib/html.js";

// OAuth code-flow providers rendered as `<a href>` redirect buttons.
const OAUTH_PROVIDER_ORDER = ["google", "github", "apple"];
// NEAR is advertised on the same `/auth/providers` list but uses a
// challenge/verify wallet handshake instead of a redirect, so it is
// surfaced separately from the OAuth redirect buttons.
const NEAR_PROVIDER = "near";

/**
 * Discover enabled login providers. Returns `{ oauth, near }`:
 * `oauth` is the ordered list of code-flow providers (rendered as
 * redirect buttons) and `near` is a boolean for the NEAR wallet button.
 */
export function useOAuthProviders() {
  const [providers, setProviders] = React.useState({ oauth: [], near: false });

  React.useEffect(() => {
    let cancelled = false;

    fetchAuthProviders()
      .then((data) => {
        if (cancelled) return;
        const enabled = Array.isArray(data?.providers) ? data.providers : [];
        setProviders({
          oauth: OAUTH_PROVIDER_ORDER.filter((provider) => enabled.includes(provider)),
          near: enabled.includes(NEAR_PROVIDER),
        });
      })
      .catch(() => {
        if (!cancelled) setProviders({ oauth: [], near: false });
      });

    return () => {
      cancelled = true;
    };
  }, []);

  return providers;
}
