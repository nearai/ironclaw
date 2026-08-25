// @ts-nocheck
import React from "react";
import { useLocation, useNavigate } from "react-router";

const CONFIGURE_QUERY_PARAM = "configure";
const SETUP_QUERY_PARAM = "setup";

// The setup paths a link may request. An unrecognized value opens the modal on
// its own choice screen rather than guessing — a newer host talking to an older
// browser must not silently land the user on the wrong ceremony.
const SETUP_PATHS = Object.freeze(["personal_account", "workspace_bot"]);

// #7853: device-link guidance rendered into a chat channel cannot show the link
// panel there — the payload it displays IS a login token, so the ceremony only
// runs in the web app. Before this the guidance named the destination in prose
// and left the user to find the Extensions page, the right extension, and the
// right path unaided.
//
// `{origin}/extensions?configure=<packageId>&setup=personal_account` closes
// that hand-off. Deliberately NOT the shape `/chat?connect=` uses: that route
// auto-installs and auto-starts a flow, which is why its notice is withheld
// from any channel that cannot deliver a reply privately (#7681). This one only
// opens a modal, so it is safe to sit in a group transcript — a bystander who
// clicks it authenticates as themselves and lands on their own page.
//
// The param is consumed once and stripped, so a reload cannot replay it, and it
// resolves against the caller's own installed inventory: an id naming something
// they do not have installed opens nothing.
//
// The resolved path is bound to the extension it named, not handed out bare:
// a caller that renders the same modal component for whatever is currently
// selected must not have a later, unrelated Configure action inherit a path
// meant for the deep link's extension. `selected` is whatever the caller is
// about to open the modal for; `clearSetupPath` should run when that modal
// closes so a later reopen of the SAME extension also lands on the choice
// screen instead of replaying the ceremony.
export function useExtensionSetupLanding({
  extensions = [],
  isLoading = false,
  onConfigure = null,
  selected = null,
}) {
  const location = useLocation();
  const navigate = useNavigate();
  // The id and the path the URL named travel as one value: they are set
  // together and consumed together, and a split pair invites clearing one.
  const [requested, setRequested] = React.useState(null);
  const [setup, setSetup] = React.useState(null);
  const landedRef = React.useRef(false);

  // First render only: `navigate` below rewrites `location.search`, so
  // re-running on later location changes would find nothing to consume.
  React.useEffect(() => {
    const params = new URLSearchParams(location.search);
    const configure = params.get(CONFIGURE_QUERY_PARAM);
    if (!configure) return;
    const setupParam = params.get(SETUP_QUERY_PARAM);
    params.delete(CONFIGURE_QUERY_PARAM);
    params.delete(SETUP_QUERY_PARAM);
    const search = params.toString();
    navigate(
      { pathname: location.pathname, search: search ? `?${search}` : "" },
      { replace: true },
    );
    setRequested({
      extensionId: configure,
      path: SETUP_PATHS.includes(setupParam) ? setupParam : null,
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Resolution waits for the inventory. The param is stripped immediately (a
  // reload must not replay it) but the extension list arrives asynchronously,
  // so the request is held until there is something to resolve it against.
  React.useEffect(() => {
    if (!requested || landedRef.current || isLoading || !onConfigure) return;
    const match = extensions.find(
      (extension) => extensionPackageId(extension) === requested.extensionId,
    );
    landedRef.current = true;
    setRequested(null);
    if (!match) {
      // Not installed for this caller. Opening nothing is correct: the
      // Extensions page they landed on is where they would install it.
      return;
    }
    setSetup(requested);
    onConfigure(match);
  }, [extensions, isLoading, onConfigure, requested]);

  const setupPath =
    setup && selected && extensionPackageId(selected) === setup.extensionId
      ? setup.path
      : null;
  const clearSetupPath = React.useCallback(() => setSetup(null), []);

  return { setupPath, clearSetupPath };
}

// Accepts either shape. The caller normalizes, but a raw API item reaching
// here must still resolve rather than silently match nothing: reading only the
// camelCase field is what made the deep link open nothing against a real
// deployment while every unit test passed on a hand-built fixture.
function extensionPackageId(extension) {
  const packageRef = extension?.packageRef ?? extension?.package_ref;
  return typeof packageRef === "string" ? packageRef : packageRef?.id || "";
}
