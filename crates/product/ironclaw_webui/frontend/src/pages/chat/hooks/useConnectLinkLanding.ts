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
  OAUTH_FLOW_POLL_MS,
  OAUTH_FLOW_STATUS_ERROR_KEYS,
  OAUTH_FLOW_TIMEOUT_MS,
  completionMatchesFlow,
  openAuthPopup,
  subscribeProductAuthOAuthCompletion,
} from "../../../lib/product-auth-oauth-events";
import {
  fetchExtensionRegistry,
  fetchExtensionSetup,
  fetchExtensions,
  fetchOauthFlowStatus,
  installExtension,
  startExtensionOauth,
} from "../../extensions/lib/extensions-api";
import { channelConnection } from "../../extensions/lib/extensions-schema";

const CONNECT_LINK_QUERY_PARAM = "connect";

function normalizeExtensionId(value) {
  return String(value || "").trim().toLowerCase();
}

// The `connect` param is attacker-controllable — it arrives on a link anyone can
// craft — so it names nothing until the server's own extension inventory
// confirms it. Resolve it against the installed list and the registry, and
// accept it only as an extension whose channel surface actually connects over
// OAuth. Anything else (unknown id, a tools-only extension, a channel that
// pairs by proof code or device link) is ignored silently: an unsolicited
// install + OAuth start is exactly what must not happen from a crafted link.
async function resolveOauthConnectTarget(extensionName) {
  const wanted = normalizeExtensionId(extensionName);
  if (!wanted) return null;
  const [installed, registry] = await Promise.all([
    Promise.resolve(fetchExtensions()).catch(() => null),
    Promise.resolve(fetchExtensionRegistry()).catch(() => null),
  ]);
  const entries = [
    ...(Array.isArray(installed) ? installed : installed?.extensions || []),
    ...(registry?.entries || []),
  ];
  const match = entries.find(
    (entry) =>
      normalizeExtensionId(entry?.package_ref?.id) === wanted &&
      channelConnection(entry)?.strategy === "oauth",
  );
  if (!match) return null;
  return {
    // The id the server published, never the raw param text.
    extensionName: match.package_ref.id,
    // The server-provided display name, so the button cannot be made to read
    // more plausibly than the target actually is.
    displayName:
      channelConnection(match)?.display_name || match.display_name || match.package_ref.id,
  };
}

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
    const requested = params.get(CONNECT_LINK_QUERY_PARAM);
    if (!requested) return undefined;
    params.delete(CONNECT_LINK_QUERY_PARAM);
    const search = params.toString();
    navigate(
      { pathname: location.pathname, search: search ? `?${search}` : "" },
      { replace: true },
    );
    let cancelled = false;
    resolveOauthConnectTarget(requested)
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
  //
  // Two signals, because neither alone is sufficient. The broadcast is the
  // fast path but is same-origin only: when the OAuth callback lands on a
  // different origin than the opener (a tunnelled callback against a
  // 127.0.0.1 app, or split app/callback domains), it never arrives and the
  // card would spin forever. Polling the durable flow status closes that gap.
  // It does NOT survive a reload mid-flow: this state is component-local and
  // the `connect` param is stripped on first render, so a reload drops the card
  // and the user reconnects from Settings/Extensions instead.
  React.useEffect(() => {
    if (!pendingFlow) return undefined;
    // One status call at a time. Without this a slow call would stack a new
    // request every tick for the whole 10-minute window, and a late resolution
    // could settle or fail a flow a newer tick already finished.
    let statusCheckInFlight = false;

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
    const timer = window.setInterval(() => {
      if (statusCheckInFlight) return;
      if (Date.now() - pendingFlow.startedAt > OAUTH_FLOW_TIMEOUT_MS) {
        fail("extensions.oauthTimedOut");
        return;
      }
      statusCheckInFlight = true;
      Promise.resolve(fetchOauthFlowStatus(pendingFlow.flowId, pendingFlow.invocationId))
        .then((result) => {
          if (flowIdRef.current !== pendingFlow.flowId) return;
          if (result?.status === "completed") {
            void settle();
            return;
          }
          const errorKey = OAUTH_FLOW_STATUS_ERROR_KEYS[result?.status];
          if (errorKey) fail(errorKey);
        })
        // A transient status-call failure is not a terminal flow outcome: keep
        // polling rather than raising an unhandled rejection every tick.
        .catch(() => null)
        .finally(() => {
          statusCheckInFlight = false;
        });
    }, OAUTH_FLOW_POLL_MS);

    return () => {
      window.clearInterval(timer);
      unsubscribe();
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
