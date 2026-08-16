// @ts-nocheck
import React from "react";
import { useLocation, useNavigate } from "react-router";
import { useT } from "../../../lib/i18n";
import { queryClient } from "../../../lib/query-client";
import {
  channelConnectionDisplayName,
  notifyChannelConnected,
} from "../../../lib/channel-connection-events";
import {
  completionMatchesFlow,
  openAuthPopup,
  subscribeProductAuthOAuthCompletion,
} from "../../../lib/product-auth-oauth-events";
import { fetchExtensionSetup, startExtensionOauth } from "../../extensions/lib/extensions-api";

const CONNECT_LINK_QUERY_PARAM = "connect";

// #7681: an OAuth-strategy channel's "connect your account" chat notice can
// carry a one-click link — `/chat?connect=<extension>`. `/chat` is an
// authenticated route, so the link rides the WebUI's existing login
// round-trip (`RequireAuth` → `redirect_after` → `login_ticket`) unchanged:
// a logged-out click lands through `/login` first, a logged-in one lands here
// directly. This hook owns the landing half only — detect the param, strip it
// so a reload cannot replay it, and drive the same
// setup → oauth-start → popup sequence the Extensions page's Connect button
// uses. No new backend route.
export function useConnectLinkLanding() {
  const t = useT();
  const location = useLocation();
  const navigate = useNavigate();
  const [connectLanding, setConnectLanding] = React.useState(null);
  const flowIdRef = React.useRef(null);

  // First render only: `navigate` below rewrites `location.search`, so
  // re-running on later location changes would find nothing to consume.
  React.useEffect(() => {
    const params = new URLSearchParams(location.search);
    const extensionName = params.get(CONNECT_LINK_QUERY_PARAM);
    if (!extensionName) return;
    params.delete(CONNECT_LINK_QUERY_PARAM);
    const search = params.toString();
    navigate(
      { pathname: location.pathname, search: search ? `?${search}` : "" },
      { replace: true },
    );
    setConnectLanding({
      extensionName,
      strategy: "oauth",
      submitLabel: t("pairing.continueConnect", {
        name: channelConnectionDisplayName(extensionName),
      }),
    });
  }, []);

  // Clear the card on real backend evidence, never optimistically on click.
  React.useEffect(() => {
    if (!connectLanding) return undefined;
    return subscribeProductAuthOAuthCompletion(window, (payload) => {
      if (!completionMatchesFlow(payload, flowIdRef.current)) return;
      flowIdRef.current = null;
      queryClient.invalidateQueries?.({ queryKey: ["extensions"] });
      notifyChannelConnected({ channel: connectLanding.extensionName, source: "connect-link" });
      setConnectLanding(null);
    });
  }, [connectLanding]);

  const startConnectLinkOAuth = React.useCallback(async () => {
    if (!connectLanding) throw new Error("connection is no longer pending");
    const packageRef = { kind: "extension", id: connectLanding.extensionName };
    // Open the placeholder popup BEFORE any await: a slow setup fetch would
    // otherwise burn the click's user activation and get the real popup
    // blocked (same reasoning as `useChannelOnboarding.startOnboardingOAuth`).
    const popup = window.open("about:blank", "_blank", "width=600,height=600");
    if (!popup) throw new Error(t("authGate.popupBlocked"));
    popup.opener = null;
    try {
      const setup = await fetchExtensionSetup(packageRef);
      const secret = (setup?.secrets || []).find(
        (item) => (item?.setup?.kind || "manual_token") === "oauth",
      );
      if (!secret) throw new Error(t("extensions.oauthSetupFailed"));
      const response = await startExtensionOauth(packageRef, secret);
      if (response?.success === false) {
        throw new Error(response.message || t("extensions.oauthSetupFailed"));
      }
      if (!response?.authorization_url || !response?.flow_id) {
        throw new Error(t("extensions.oauthSetupFailed"));
      }
      const opened = openAuthPopup(response.authorization_url, popup);
      if (!opened.ok) {
        throw new Error(
          opened.reason === "popup_blocked"
            ? t("authGate.popupBlocked")
            : t("extensions.oauthInvalidAuthorizationUrl"),
        );
      }
      flowIdRef.current = response.flow_id;
      return response;
    } catch (error) {
      if (!popup.closed) popup.close();
      throw error;
    }
  }, [connectLanding, t]);

  const dismissConnectLanding = React.useCallback(() => {
    flowIdRef.current = null;
    setConnectLanding(null);
  }, []);

  return { connectLanding, startConnectLinkOAuth, dismissConnectLanding };
}
