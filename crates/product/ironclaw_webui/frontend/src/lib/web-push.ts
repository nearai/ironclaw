// @ts-nocheck
// Browser-side Web Push enrollment for the web-push notification channel.
//
// The service worker (`public/sw.js`) displays pushes; this module owns
// registration and the per-browser subscription lifecycle against the
// `/api/webchat/v2/web-push/*` routes. All entry points are defensive:
// missing browser APIs degrade to an "unsupported" state and never throw
// into app boot or the settings panel.

import { subscribeWebPush, unsubscribeWebPush } from "./api";

const SERVICE_WORKER_URL = "/sw.js";

/** Register the notification service worker. Safe to call on every boot;
 * failures are logged and swallowed so app startup never depends on it. */
export function registerServiceWorker() {
  if (typeof navigator === "undefined" || !("serviceWorker" in navigator)) {
    return Promise.resolve(null);
  }
  return navigator.serviceWorker
    .register(SERVICE_WORKER_URL)
    .catch((error) => {
      console.warn("IronClaw service worker registration failed", error);
      return null;
    });
}

function pushSupported() {
  return (
    typeof navigator !== "undefined" &&
    "serviceWorker" in navigator &&
    typeof window !== "undefined" &&
    "PushManager" in window &&
    typeof Notification !== "undefined"
  );
}

async function pushRegistration() {
  // `ready` resolves once the boot-time registration activates; it never
  // rejects for a registered worker, but guard a missing registration in
  // browsers that lost it (e.g. cleared site data mid-session).
  const registration = await navigator.serviceWorker.ready;
  if (!registration || !registration.pushManager) return null;
  return registration;
}

/**
 * The current browser's push state:
 *   { state: "unsupported" }
 *   { state: "permission-denied" }
 *   { state: "not-enrolled" }
 *   { state: "enrolled", endpoint }
 */
export async function getWebPushBrowserState() {
  if (!pushSupported()) return { state: "unsupported" };
  if (Notification.permission === "denied") return { state: "permission-denied" };
  try {
    const registration = await pushRegistration();
    if (!registration) return { state: "unsupported" };
    const subscription = await registration.pushManager.getSubscription();
    if (subscription && subscription.endpoint) {
      return { state: "enrolled", endpoint: subscription.endpoint };
    }
    return { state: "not-enrolled" };
  } catch (_) {
    return { state: "not-enrolled" };
  }
}

/** Decode an unpadded base64url VAPID public key into the byte array
 * `pushManager.subscribe` expects. */
export function urlBase64ToUint8Array(base64String) {
  if (typeof base64String !== "string" || !base64String) {
    throw new Error("a base64url key is required");
  }
  const padding = "=".repeat((4 - (base64String.length % 4)) % 4);
  const base64 = (base64String + padding).replace(/-/g, "+").replace(/_/g, "/");
  const raw = atob(base64);
  const output = new Uint8Array(raw.length);
  for (let index = 0; index < raw.length; index += 1) {
    output[index] = raw.charCodeAt(index);
  }
  return output;
}

function subscriptionKeys(subscription) {
  const json = subscription.toJSON ? subscription.toJSON() : {};
  const keys = json.keys || {};
  if (!keys.p256dh || !keys.auth) {
    throw new Error("push subscription did not expose p256dh/auth keys");
  }
  return { p256dh: keys.p256dh, auth: keys.auth };
}

/** Ask for permission, subscribe this browser, and register the
 * subscription with the backend. Returns the resulting browser state. */
export async function enrollThisBrowser({ vapidPublicKey } = {}) {
  if (!vapidPublicKey) {
    throw new Error("vapidPublicKey is required");
  }
  if (!pushSupported()) return { state: "unsupported" };
  const permission = await Notification.requestPermission();
  if (permission !== "granted") {
    return Notification.permission === "denied"
      ? { state: "permission-denied" }
      : { state: "not-enrolled" };
  }
  const registration = await pushRegistration();
  if (!registration) return { state: "unsupported" };
  let subscription = await registration.pushManager.getSubscription();
  if (!subscription) {
    subscription = await registration.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: urlBase64ToUint8Array(vapidPublicKey),
    });
  }
  await subscribeWebPush({
    endpoint: subscription.endpoint,
    keys: subscriptionKeys(subscription),
    userAgent: typeof navigator !== "undefined" ? navigator.userAgent : undefined,
  });
  return { state: "enrolled", endpoint: subscription.endpoint };
}

/** Unsubscribe this browser locally and remove it from the backend.
 * Returns the resulting browser state. */
export async function unenrollThisBrowser() {
  if (!pushSupported()) return { state: "unsupported" };
  const registration = await pushRegistration();
  if (!registration) return { state: "unsupported" };
  const subscription = await registration.pushManager.getSubscription();
  if (!subscription) return { state: "not-enrolled" };
  const endpoint = subscription.endpoint;
  await subscription.unsubscribe();
  // Backend removal is best-effort: the dead subscription is also pruned
  // server-side on the next 404/410 push response.
  try {
    await unsubscribeWebPush({ endpoint });
  } catch (error) {
    console.warn("web push backend unsubscribe failed", error);
  }
  return { state: "not-enrolled" };
}
