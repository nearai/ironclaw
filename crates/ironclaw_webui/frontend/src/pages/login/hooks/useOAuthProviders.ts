import { fetchAuthProviders } from "../../../lib/api";
import React from "react";

const OAUTH_PROVIDER_ORDER = ["google", "github", "apple"];

export function useOAuthProviders() {
  const [providers, setProviders] = React.useState([]);

  React.useEffect(() => {
    let cancelled = false;

    fetchAuthProviders()
      .then((data) => {
        if (cancelled) return;
        const enabled = Array.isArray(data?.providers) ? data.providers : [];
        setProviders(OAUTH_PROVIDER_ORDER.filter((provider) => enabled.includes(provider)));
      })
      .catch(() => {
        if (!cancelled) setProviders([]);
      });

    return () => {
      cancelled = true;
    };
  }, []);

  return providers;
}
