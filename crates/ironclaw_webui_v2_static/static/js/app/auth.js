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
    // Local-dev with no profile defaults to admin so the fork's
    // sidebar / admin pages render. Matches the fork's `!profile`
    // permissive read — production deployments that surface a real
    // profile through a future v2 endpoint can flip this.
    isAdmin: true,
    signIn,
    signOut,
  };
}
