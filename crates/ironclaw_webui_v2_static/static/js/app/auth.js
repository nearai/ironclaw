import { React } from "../lib/html.js";
import { queryClient } from "../lib/query-client.js";
import {
  exchangeLoginTicket,
  logout as logoutRequest,
  readStoredToken,
  storeToken,
} from "../lib/api.js";

// The Reborn host validates bearer tokens via OIDC; the SPA simply
// carries whatever token the user supplies (via `?token=` URL param,
// `#token=` URL fragment, OAuth `login_ticket` exchange, or
// `sessionStorage`) and lets the server reject anything invalid. No
// v2 endpoint exposes session probing or profile info, so this hook
// holds no derived identity state.
//
// `?token=`  — manual-token paste pattern (the "Connect" form on
//              the login page).
// `login_ticket=` — OAuth callback transport. The host's
//              `/auth/callback/{provider}` redirects to
//              `/v2?login_ticket=<ticket>`. The ticket is short-lived
//              and single-use; the SPA POSTs it to
//              `/auth/session/exchange` for the real bearer so the
//              bearer never appears in a redirect `Location` header.
//
// Either form is honored ONLY when sessionStorage has no token yet.
// Without this guard a crafted `/v2/#token=INVALID` link could
// replace a user's working bearer with garbage and lock them out
// until they re-auth. The token is always stripped from the URL
// (query AND fragment) so a copy-paste of the address bar does not
// leak it onward.
function readFragmentParam(hash, name) {
  if (!hash) return "";
  // location.hash starts with "#". Treat the rest as a urlencoded
  // key=value list so `#token=abc&login=ok` round-trips through
  // URLSearchParams cleanly.
  const stripped = hash.startsWith("#") ? hash.slice(1) : hash;
  try {
    return new URLSearchParams(stripped).get(name) || "";
  } catch (_) {
    return "";
  }
}

function stripFragmentParam(hash, name) {
  if (!hash) return "";
  const stripped = hash.startsWith("#") ? hash.slice(1) : hash;
  try {
    const params = new URLSearchParams(stripped);
    params.delete(name);
    const remainder = params.toString();
    return remainder ? `#${remainder}` : "";
  } catch (_) {
    return hash;
  }
}

function consumeTokenFromUrl() {
  const url = new URL(window.location.href);
  const queryToken = (url.searchParams.get("token") || "").trim();
  const fragmentToken = readFragmentParam(url.hash, "token").trim();
  // Fragment wins for manual/debug URLs so `#token=` can override a
  // stale `?token=` left over in the address bar. OAuth uses
  // `login_ticket`, not a bearer fragment.
  const token = fragmentToken || queryToken;
  if (!token) return "";

  // Always strip the token from query AND fragment — leaving it in
  // the address bar would let a copy-paste leak it onward even
  // after we've consumed it for this session.
  if (queryToken) url.searchParams.delete("token");
  const newHash = fragmentToken ? stripFragmentParam(url.hash, "token") : url.hash;
  window.history.replaceState({}, "", url.pathname + url.search + newHash);

  if (readStoredToken()) {
    // A stored token already exists — refuse to overwrite it. The
    // user is logged in; an unsolicited token is either stale,
    // adversarial, or a stray reload of a one-time link.
    return "";
  }
  storeToken(token);
  return token;
}

function consumeLoginTicketFromUrl() {
  const url = new URL(window.location.href);
  const ticket = (url.searchParams.get("login_ticket") || "").trim();
  if (!ticket) return "";
  url.searchParams.delete("login_ticket");
  window.history.replaceState({}, "", url.pathname + url.search + url.hash);
  return ticket;
}

// Map opaque error codes the OAuth callback emits (`?login_error=...`)
// to short user-facing messages. Keeps the SPA's surface in sync with
// `error_code_for` in `crates/ironclaw_reborn_webui_ingress/src/auth/routes.rs`.
// An unknown code falls back to a generic message so a future
// backend addition does not render a blank banner.
const LOGIN_ERROR_MESSAGES = {
  denied: "Sign-in was cancelled.",
  invalid_state: "Your sign-in session expired. Please try again.",
  invalid_request: "Sign-in request was malformed. Please try again.",
  provider_mismatch: "Sign-in provider mismatch. Please try again.",
  unauthorized: "This account is not authorized.",
  exchange_failed: "Could not complete sign-in with the provider.",
  server_error: "Sign-in is temporarily unavailable.",
};

// Read `?login_error=<code>` from the URL once on mount and strip
// it. Returns the localized banner text (empty string if no code or
// the code is unknown).
function consumeLoginErrorFromUrl() {
  const url = new URL(window.location.href);
  const code = (url.searchParams.get("login_error") || "").trim();
  if (!code) return "";
  url.searchParams.delete("login_error");
  window.history.replaceState({}, "", url.pathname + url.search + url.hash);
  return LOGIN_ERROR_MESSAGES[code] || "Could not complete sign-in. Please try again.";
}

export function useAuthSession() {
  const [token, setToken] = React.useState(
    () => consumeTokenFromUrl() || readStoredToken(),
  );
  const [error, setError] = React.useState(() => consumeLoginErrorFromUrl());
  const [loginTicket] = React.useState(() => consumeLoginTicketFromUrl());
  const [isExchanging, setIsExchanging] = React.useState(
    () => Boolean(loginTicket && !readStoredToken()),
  );

  React.useEffect(() => {
    if (!loginTicket || readStoredToken()) {
      setIsExchanging(false);
      return undefined;
    }
    let cancelled = false;
    exchangeLoginTicket(loginTicket)
      .then((nextToken) => {
        if (cancelled) return;
        storeToken(nextToken);
        setToken(nextToken);
        setError("");
        setIsExchanging(false);
        queryClient.clear();
      })
      .catch(() => {
        if (cancelled) return;
        setError("Could not complete sign-in. Please try again.");
        setIsExchanging(false);
      });
    return () => {
      cancelled = true;
    };
  }, [loginTicket]);

  const signIn = React.useCallback((nextToken) => {
    storeToken(nextToken);
    setToken(nextToken);
    setError("");
    queryClient.clear();
  }, []);

  const signOut = React.useCallback(() => {
    // Fire-and-forget the server-side revoke so a refresh or another
    // tab cannot keep using the bearer. The local clear is
    // unconditional: even if the request fails (network glitch,
    // backend down) the SPA still drops the token so the user is
    // visually signed out and can re-authenticate.
    logoutRequest().catch(() => {});
    storeToken("");
    setToken("");
    setError("");
    queryClient.clear();
  }, []);

  return {
    token,
    profile: null,
    error,
    setError,
    isChecking: isExchanging,
    isAuthenticated: Boolean(token),
    // No v2 profile endpoint exists yet, so the SPA cannot prove
    // admin status — default closed. The fork's `!profile`
    // permissive read defaulted open, which is the wrong direction
    // for a bearer-only auth surface. Admin-gated routes are also
    // hidden via `route.hidden`, so this is defense in depth; once a
    // server-issued profile endpoint lands the flag flips from
    // there.
    isAdmin: false,
    signIn,
    signOut,
  };
}
