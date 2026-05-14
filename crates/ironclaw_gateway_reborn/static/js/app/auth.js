import { React } from "../lib/html.js";
import { queryClient } from "../lib/query-client.js";
import {
  fetchProfile,
  fetchThreads,
  logoutSession,
  readStoredToken,
  storeToken,
} from "../lib/api.js";

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
  const [token, setToken] = React.useState(() => consumeTokenFromUrl() || readStoredToken());
  const [profile, setProfile] = React.useState(null);
  const [hasImplicitSession, setHasImplicitSession] = React.useState(false);
  const [isChecking, setIsChecking] = React.useState(true);
  const [error, setError] = React.useState("");

  React.useEffect(() => {
    let cancelled = false;
    setIsChecking(true);

    fetchProfile()
      .then((nextProfile) => {
        if (cancelled) return;
        setProfile(nextProfile || null);
        setHasImplicitSession(true);
        setError("");
      })
      .catch(async () => {
        if (cancelled) return;
        setProfile(null);
        setHasImplicitSession(false);

        if (!token) {
          try {
            await fetchThreads();
            if (!cancelled) setHasImplicitSession(true);
          } catch (_) {}
        }
      })
      .finally(() => {
        if (!cancelled) setIsChecking(false);
      });

    return () => {
      cancelled = true;
    };
  }, [token]);

  const signIn = React.useCallback((nextToken) => {
    storeToken(nextToken);
    setToken(nextToken);
    setProfile(null);
    setHasImplicitSession(false);
    setError("");
    queryClient.clear();
  }, []);

  const signOut = React.useCallback(async () => {
    try {
      await logoutSession();
    } catch (_) {}
    storeToken("");
    sessionStorage.removeItem("ironclaw_oidc");
    setToken("");
    setProfile(null);
    setHasImplicitSession(false);
    setError("");
    queryClient.clear();
  }, []);

  return {
    token,
    profile,
    error,
    setError,
    isChecking,
    isAuthenticated: Boolean(token) || Boolean(profile) || hasImplicitSession,
    isAdmin: !profile || profile.role === "admin",
    signIn,
    signOut,
  };
}
