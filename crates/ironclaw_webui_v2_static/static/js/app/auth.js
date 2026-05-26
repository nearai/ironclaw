import { React } from "../lib/html.js";
import { queryClient } from "../lib/query-client.js";
import { readStoredToken, storeToken } from "../lib/api.js";

// The Reborn host validates bearer tokens via OIDC; the SPA simply
// carries whatever token the user supplies (via `?token=` URL param
// or `sessionStorage`) and lets the server reject anything invalid.
// No v2 endpoint exposes session probing or profile info, so this
// hook holds no derived identity state.
function consumeTokenFromUrl() {
  const url = new URL(window.location.href);
  const token = (url.searchParams.get("token") || "").trim();
  if (!token) return "";

  url.searchParams.delete("token");
  window.history.replaceState({}, "", url.pathname + url.search + url.hash);
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
