import { React } from "../lib/html.js";
import { queryClient } from "../lib/query-client.js";
import { logout as logoutRequest, readStoredToken, storeToken } from "../lib/api.js";

// The Reborn host validates bearer tokens via OIDC; the SPA simply
// carries whatever token the user supplies (via `?token=` URL param,
// `#token=` URL fragment, or `sessionStorage`) and lets the server
// reject anything invalid. No v2 endpoint exposes session probing
// or profile info, so this hook holds no derived identity state.
//
// `?token=`  — manual-token paste pattern (the "Connect" form on
//              the login page).
// `#token=`  — OAuth callback transport. The host's
//              `/auth/callback/{provider}` redirects to
//              `/v2#token=<bearer>`. Fragments are never sent to
//              the server in subsequent navigation, so the bearer
//              cannot leak through HTTP access logs or `Referer`
//              headers.
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
  const token = fragmentToken || queryToken;
  if (!token && !queryToken && !fragmentToken) {
    return "";
  }

  // Always strip the token from query AND fragment, even if we
  // won't use it — leaving it in the address bar would let a
  // copy-paste leak the token regardless of whether it ends up
  // authenticating this session.
  if (queryToken) url.searchParams.delete("token");
  const newHash = fragmentToken ? stripFragmentParam(url.hash, "token") : url.hash;
  window.history.replaceState({}, "", url.pathname + url.search + newHash);

  if (!token) return "";

  if (readStoredToken()) {
    // A stored token already exists — refuse to overwrite it. The
    // user is logged in; an unsolicited token is either stale,
    // adversarial, or a stray reload of a one-time link.
    return "";
  }
  storeToken(token);
  return token;
}

export function useAuthSession() {
  const [token, setToken] = React.useState(
    () => consumeTokenFromUrl() || readStoredToken(),
  );
  const [error, setError] = React.useState("");

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
    isChecking: false,
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
