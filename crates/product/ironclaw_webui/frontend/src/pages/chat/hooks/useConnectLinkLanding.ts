// @ts-nocheck
import React from "react";
import { useLocation, useNavigate } from "react-router";
import { useT } from "../../../lib/i18n";
import { queryClient } from "../../../lib/query-client";
import {
  channelConnectionDisplayName,
  notifyChannelConnected,
} from "../../../lib/channel-connection-events";

const CONNECT_LINK_QUERY_PARAM = "connect";

// The resolver, the install/oauth-start sequence, and the flow watcher all live
// in `../lib/connect-link-flow`, loaded through this dynamic `import()` only
// when a `connect` param is actually on the URL. Almost nobody lands on /chat
// with one, and the eager chat chunk has a gzip budget
// (`scripts/check-bundle-budgets.ts`), so that machinery — plus the extensions
// API client and surface schema it pulls in — stays out of the initial route.
function loadConnectLinkFlow() {
  return import("../lib/connect-link-flow");
}

// #7681: an OAuth-strategy channel's "connect your account" chat notice can
// carry a one-click link — `/chat?connect=<extension>`. `/chat` is an
// authenticated route, so the link rides the WebUI's existing login
// round-trip (`RequireAuth` → `redirect_after` → `login_ticket`) unchanged:
// a logged-out click lands through `/login` first, a logged-in one lands here
// directly. This hook owns the landing half only — detect the param, strip it
// so a reload cannot replay it, resolve it against the server's own extension
// inventory, and drive the same setup → oauth-start → popup sequence the
// Extensions page's Connect button uses. No new backend route.
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
    const requested = params.get(CONNECT_LINK_QUERY_PARAM);
    if (!requested) return undefined;
    params.delete(CONNECT_LINK_QUERY_PARAM);
    const search = params.toString();
    navigate(
      { pathname: location.pathname, search: search ? `?${search}` : "" },
      { replace: true },
    );
    let cancelled = false;
    loadConnectLinkFlow()
      .then(({ resolveOauthConnectTarget }) => resolveOauthConnectTarget(requested))
      .then((target) => {
        if (cancelled || !target) return;
        setConnectLanding({
          extensionName: target.extensionName,
          strategy: "oauth",
          submitLabel: t("pairing.continueConnect", {
            name: channelConnectionDisplayName(target.extensionName, target.displayName),
          }),
        });
      })
      .catch(() => null);
    return () => {
      cancelled = true;
    };
  }, []);

  // Clear the card on real backend evidence, never optimistically on click.
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

    let stopWatching = null;
    let cancelled = false;
    // Already resolved by the time a flow exists — the click that started the
    // flow loaded this module — so the watcher attaches on the next microtask.
    loadConnectLinkFlow()
      .then(({ watchConnectLinkFlow }) => {
        if (cancelled) return;
        stopWatching = watchConnectLinkFlow({
          flow: pendingFlow,
          browserWindow: window,
          isCurrent: () => flowIdRef.current === pendingFlow.flowId,
          onCompleted: () => void settle(),
          onFailed: fail,
        });
      })
      .catch(() => null);

    return () => {
      cancelled = true;
      stopWatching?.();
    };
  }, [pendingFlow, t]);

  const startConnectLinkOAuth = React.useCallback(async () => {
    if (!connectLanding) throw new Error("connection is no longer pending");
    // A retry after a failed attempt clears the stale card error first. The card
    // only leaves its spinner when the `oauthError` VALUE changes, so without
    // this a second identical failure would produce no prop change and the card
    // would spin forever.
    setConnectLanding((current) =>
      current?.oauthError ? { ...current, oauthError: null } : current,
    );
    // Open the placeholder popup BEFORE any await: a slow module load or setup
    // fetch would otherwise burn the click's user activation and get the real
    // popup blocked (same reasoning as
    // `useChannelOnboarding.startOnboardingOAuth`).
    const popup = window.open("about:blank", "_blank", "width=600,height=600");
    if (!popup) throw new Error(t("authGate.popupBlocked"));
    popup.opener = null;
    try {
      const { startConnectLinkOauth } = await loadConnectLinkFlow();
      const { response, flow } = await startConnectLinkOauth({
        extensionName: connectLanding.extensionName,
        popup,
        t,
      });
      flowIdRef.current = flow.flowId;
      setPendingFlow(flow);
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
