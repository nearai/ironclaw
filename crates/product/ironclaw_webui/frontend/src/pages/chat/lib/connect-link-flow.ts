// @ts-nocheck
//
// The machinery behind the `/chat?connect=<extension>` landing card: resolving
// the untrusted param against the server's extension inventory, driving the
// install -> setup -> oauth-start sequence, and watching the flow to completion.
//
// This module is loaded through a dynamic `import()` from
// `hooks/useConnectLinkLanding.ts`, and only when a `connect` param is actually
// present on the URL. That keeps the extensions API client, the extensions
// surface schema, and the OAuth watcher out of the eager /chat chunk, which is
// budgeted (`scripts/check-bundle-budgets.ts`). The hook keeps the param
// detection, the URL strip, and the card state — all tiny — inline.
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
export async function resolveOauthConnectTarget(extensionName) {
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

// Drive the same setup -> oauth-start -> popup sequence the Extensions page's
// Connect button uses, pointing the caller's already-open placeholder popup at
// the authorization URL. The caller opens that popup synchronously on the click
// so a slow fetch cannot burn the user activation.
export async function startConnectLinkOauth({ extensionName, popup, t }) {
  const packageRef = { kind: "extension", id: extensionName };
  // Install first. Someone arriving from a channel nudge has, by definition,
  // not connected this extension — and usually has not installed it either,
  // which makes `setup/oauth/start` fail closed (`require_installed_extension`
  // -> 409). Install is idempotent, so an already-installed extension costs one
  // no-op call rather than a pre-flight inventory read.
  // A rejected install is reported in the response, not as a throw, so read
  // the backend's own verdict before continuing: without this the flow would
  // walk on to setup and OAuth start for an extension that was never
  // installed, and surface the resulting 409 as an OAuth failure.
  const installation = await installExtension(packageRef);
  if (installation?.success === false) {
    throw new Error(installation.message || t("extensions.installFailed"));
  }
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
  return {
    response,
    flow: {
      flowId: response.flow_id,
      // The caller-scoped backend needs this to locate its own flow when
      // reconciling status; absent on responses that mint no callback scope.
      invocationId:
        response?.callback_scope?.invocation_id ||
        response?.callbackScope?.invocationId ||
        null,
      channel: extensionName,
      startedAt: Date.now(),
    },
  };
}

// Watch a started flow to a terminal outcome and return the teardown.
//
// Two signals, because neither alone is sufficient. The broadcast is the fast
// path but is same-origin only: when the OAuth callback lands on a different
// origin than the opener (a tunnelled callback against a 127.0.0.1 app, or
// split app/callback domains), it never arrives and the card would spin
// forever. Polling the durable flow status closes that gap. It does NOT survive
// a reload mid-flow: the card state is component-local and the `connect` param
// is stripped on first render, so a reload drops the card and the user
// reconnects from Settings/Extensions instead.
export function watchConnectLinkFlow({
  flow,
  browserWindow,
  isCurrent,
  onCompleted,
  onFailed,
}) {
  // One status call at a time. Without this a slow call would stack a new
  // request every tick for the whole 10-minute window, and a late resolution
  // could settle or fail a flow a newer tick already finished.
  let statusCheckInFlight = false;

  const unsubscribe = subscribeProductAuthOAuthCompletion(browserWindow, (payload) => {
    if (isCurrent() && completionMatchesFlow(payload, flow.flowId)) onCompleted();
  });
  const timer = browserWindow.setInterval(() => {
    if (statusCheckInFlight) return;
    if (Date.now() - flow.startedAt > OAUTH_FLOW_TIMEOUT_MS) {
      onFailed("extensions.oauthTimedOut");
      return;
    }
    statusCheckInFlight = true;
    Promise.resolve(fetchOauthFlowStatus(flow.flowId, flow.invocationId))
      .then((result) => {
        if (!isCurrent()) return;
        if (result?.status === "completed") {
          onCompleted();
          return;
        }
        const errorKey = OAUTH_FLOW_STATUS_ERROR_KEYS[result?.status];
        if (errorKey) onFailed(errorKey);
      })
      // A transient status-call failure is not a terminal flow outcome: keep
      // polling rather than raising an unhandled rejection every tick.
      .catch(() => null)
      .finally(() => {
        statusCheckInFlight = false;
      });
  }, OAUTH_FLOW_POLL_MS);

  return () => {
    browserWindow.clearInterval(timer);
    unsubscribe();
  };
}
