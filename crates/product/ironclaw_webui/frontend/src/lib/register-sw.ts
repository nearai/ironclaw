// @ts-nocheck
// Boot-time service-worker registration, isolated from the web-push
// enrollment lib on purpose. `main.tsx` calls this on every startup, so it
// must stay dependency-free — pulling in the enrollment machinery
// (`web-push.ts`, which imports the api client + WebCrypto digest helpers)
// would drag all of it into the initial `/chat` bundle, which the bundle
// budget gate rejects. The automations-page enrollment API lives in
// `web-push.ts` and is imported only by that route's hook.

const SERVICE_WORKER_URL = "/sw.js";

/** Register the notification service worker. Safe to call on every boot;
 * failures are logged and swallowed so app startup never depends on it. */
export function registerServiceWorker() {
  if (typeof navigator === "undefined" || !("serviceWorker" in navigator)) {
    return Promise.resolve(null);
  }
  return navigator.serviceWorker.register(SERVICE_WORKER_URL).catch((error) => {
    console.warn("IronClaw service worker registration failed", error);
    return null;
  });
}
