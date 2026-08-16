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
import {
  fetchExtensionSetup,
  fetchOauthFlowStatus,
  installExtension,
  startExtensionOauth,
} from "../../extensions/lib/extensions-api";

const CONNECT_LINK_QUERY_PARAM = "connect";
// Watcher bounds, mirroring the in-chat onboarding watcher so an abandoned
// popup cannot leave the card polling forever.
const CONNECT_OAUTH_TIMEOUT_MS = 10 * 60 * 1000;
const CONNECT_OAUTH_POLL_MS = 2000;
const CONNECT_OAUTH_STATUS_ERROR_KEYS = Object.freeze({
  failed: "extensions.oauthFailed",
  canceled: "extensions.oauthCanceled",
  expired: "extensions.oauthExpired",
});

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
  const [pendingFlow, setPendingFlow] = React.useState(null);
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
  //
  // Two signals, because neither alone is sufficient. The broadcast is the
  // fast path but is same-origin only: when the OAuth callback lands on a
  // different origin than the opener (a tunnelled callback against a
  // 127.0.0.1 app, or split app/callback domains), it never arrives and the
  // card would spin forever. Polling the durable flow status closes that gap
  // and is also what recovers the card after a reload mid-flow.
  React.useEffect(() => {
    if (!pendingFlow) return undefined;

    const settle = async () => {
      if (flowIdRef.current !== pendingFlow.flowId) return;
      flowIdRef.current = null;
      setPendingFlow(null);
      queryClient.invalidateQueries?.({ queryKey: ["extensions"] });
      await notifyChannelConnected({
        channel: pendingFlow.channel,
        source: "connect-link",
      });
      setConnectLanding(null);
    };
    const fail = (messageKey) => {
      if (flowIdRef.current !== pendingFlow.flowId) return;
      flowIdRef.current = null;
      setPendingFlow(null);
      // The card exits its spinner and shows a retry-able error when the
      // onboarding it renders carries `oauthError`.
      setConnectLanding((current) =>
        current ? { ...current, oauthError: t(messageKey) } : current,
      );
    };

    const unsubscribe = subscribeProductAuthOAuthCompletion(window, (payload) => {
      if (completionMatchesFlow(payload, flowIdRef.current)) void settle();
    });
    const timer = window.setInterval(async () => {
      if (Date.now() - pendingFlow.startedAt > CONNECT_OAUTH_TIMEOUT_MS) {
        fail("extensions.oauthTimedOut");
        return;
      }
      const result = await fetchOauthFlowStatus(pendingFlow.flowId, pendingFlow.invocationId);
      if (result?.status === "completed") {
        void settle();
        return;
      }
      const errorKey = CONNECT_OAUTH_STATUS_ERROR_KEYS[result?.status];
      if (errorKey) fail(errorKey);
    }, CONNECT_OAUTH_POLL_MS);

    return () => {
      window.clearInterval(timer);
      unsubscribe();
    };
  }, [pendingFlow, t]);

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
      // Install first. Someone arriving from a channel nudge has, by
      // definition, not connected this extension — and usually has not
      // installed it either, which makes `setup/oauth/start` fail closed
      // (`require_installed_extension` -> 409). Install is idempotent, so an
      // already-installed extension costs one no-op call rather than a
      // pre-flight inventory read.
      await installExtension(packageRef);
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
      setPendingFlow({
        flowId: response.flow_id,
        // The caller-scoped backend needs this to locate its own flow when
        // reconciling status; absent on responses that mint no callback scope.
        invocationId:
          response?.callback_scope?.invocation_id ||
          response?.callbackScope?.invocationId ||
          null,
        channel: connectLanding.extensionName,
        startedAt: Date.now(),
      });
      return response;
    } catch (error) {
      if (!popup.closed) popup.close();
      throw error;
    }
  }, [connectLanding, t]);

  const dismissConnectLanding = React.useCallback(() => {
    flowIdRef.current = null;
    setPendingFlow(null);
    setConnectLanding(null);
  }, []);

  return { connectLanding, startConnectLinkOAuth, dismissConnectLanding };
}
