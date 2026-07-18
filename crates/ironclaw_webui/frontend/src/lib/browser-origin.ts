// Shared browser-origin detection. Lives here (not in a single page's
// hooks/ folder) because more than one page needs it: Settings' NEAR AI
// login flow (`useProviderLogin`) and the login page's local-install hint
// (`login-page.tsx`) both gate on whether the app is being served from a
// loopback/local-dev origin.

// NEAR AI's hosted auth (private.near.ai) rejects `frontend_callback` URLs that
// point at a loopback host, so its browser sign-in (GitHub / Google / NEAR
// Wallet) cannot complete from a local dev origin. Detect that origin so we can
// fail fast with a clear message on click — instead of opening a doomed tab and
// polling for five minutes only to hit the opaque error (issue #4705).
//
// Also used by the login page to decide whether the "no SSO configured"
// state means a local single-user desktop install (show the CLI hint) or a
// hosted token-only deployment being viewed from a non-local origin (no
// hint — a remote user has no use for a CLI command they can't run).
export function isLocalDevOrigin() {
  if (typeof window === "undefined" || !window.location) return false;
  const host = window.location.hostname;
  // `window.location.hostname` exposes IPv6 hosts without brackets (e.g.
  // `http://[::1]:3000/` -> `"::1"`), so a bracketed `"[::1]"` form never
  // appears here.
  //
  // The entire `127.0.0.0/8` block is loopback, not just `127.0.0.1` — some
  // setups serve the dev UI on `127.0.1.1` (Debian's default for the hostname)
  // or other `127.*` addresses. Matching only `127.0.0.1` would let those
  // origins open the doomed hosted-SSO flow and wait out the full timeout
  // instead of failing fast.
  return (
    host === "localhost" ||
    host === "0.0.0.0" ||
    host === "::1" ||
    /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(host) ||
    host.endsWith(".localhost")
  );
}
