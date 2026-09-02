// Browser-side push enrollment for the deployment's session channel — the
// channel this SPA itself fronts, learned from `GET /session` (never a
// hardcoded channel name).
//
// The service worker (`public/sw.js`) displays pushes; this module owns
// registration and the per-browser subscription lifecycle against the
// generic notification-setup surface
// (`/api/webchat/v2/channels/{extension_id}/notifications/*`). The
// enable/disable payloads and the status `detail` are that channel's own
// opaque documents — this module is the client half that interprets them.
// All entry points are defensive: missing browser APIs degrade to an
// "unsupported" state and never throw into app boot or the settings panel.

import { getSessionChannelExtensionId } from "./api";
import {
  disableNotificationSetup,
  enableNotificationSetup,
  type NotificationSetupStatusResponse,
} from "./notification-setup-api";

// `registerServiceWorker` lives in the dependency-free `./register-sw` module
// so app boot (`main.tsx`) does not pull this enrollment lib — and its api +
// WebCrypto imports — into the initial `/chat` bundle. Re-exported here so the
// automations-page hook keeps one import site for the device-push surface.
export { registerServiceWorker } from "./register-sw";

export type DevicePushState =
  | { state: "unsupported" }
  | { state: "permission-denied" }
  | { state: "not-enrolled" }
  | { state: "enrolled"; endpoint: string; accountMatch: boolean | null };

function pushSupported(): boolean {
  return (
    typeof navigator !== "undefined" &&
    "serviceWorker" in navigator &&
    typeof window !== "undefined" &&
    "PushManager" in window &&
    typeof Notification !== "undefined"
  );
}

async function pushRegistration(): Promise<ServiceWorkerRegistration | null> {
  // Deliberately `getRegistration()`, never `serviceWorker.ready`: `ready`
  // resolves only once SOME registration activates and never rejects, so
  // after a failed boot registration (non-secure origin, /sw.js 404,
  // private mode) it hangs forever and every state probe hangs with it.
  // `getRegistration()` resolves promptly with `undefined` in exactly those
  // cases, which callers surface as "unsupported".
  if (!navigator.serviceWorker || typeof navigator.serviceWorker.getRegistration !== "function") {
    return null;
  }
  try {
    const registration = await navigator.serviceWorker.getRegistration();
    if (!registration || !registration.pushManager) return null;
    return registration;
  } catch (_) {
    return null;
  }
}

/** Lowercase hex SHA-256 of the endpoint URL string — the correlation key
 * the channel's setup-status `detail` exposes as `endpoint_digest`, so the
 * browser can tell whether ITS subscription belongs to the signed-in account
 * without the backend ever echoing full endpoint capability URLs. Returns
 * null when WebCrypto is unavailable (push itself requires a secure context,
 * so this is effectively test-environment-only). */
export async function endpointDigestHex(endpoint: unknown): Promise<string | null> {
  if (typeof endpoint !== "string" || !endpoint) return null;
  const subtle = globalThis.crypto && globalThis.crypto.subtle;
  if (!subtle) return null;
  try {
    const bytes = new TextEncoder().encode(endpoint);
    const digest = await subtle.digest("SHA-256", bytes);
    return Array.from(new Uint8Array(digest))
      .map((byte) => byte.toString(16).padStart(2, "0"))
      .join("");
  } catch (_) {
    return null;
  }
}

/**
 * The current browser's push state:
 *   { state: "unsupported" }
 *   { state: "permission-denied" }
 *   { state: "not-enrolled" }
 *   { state: "enrolled", endpoint, accountMatch }
 *
 * `accountMatch` correlates the browser-global subscription with the
 * SIGNED-IN account's enrollment set (`accountEndpointDigests`, the
 * `endpoint_digest` values from the setup-status `detail`):
 *   true  — this account holds a record for this browser's subscription;
 *   false — a subscription exists but belongs to no record of this account
 *           (typically another account enrolled in this browser profile);
 *   null  — correlation unavailable (no digest list supplied, or WebCrypto
 *           missing).
 * Callers must only offer a local unsubscribe when `accountMatch === true`;
 * anything else risks severing another account's enrollment.
 */
export async function getDevicePushState({
  accountEndpointDigests = null,
}: { accountEndpointDigests?: readonly string[] | null } = {}): Promise<DevicePushState> {
  if (!pushSupported()) return { state: "unsupported" };
  if (Notification.permission === "denied") return { state: "permission-denied" };
  try {
    const registration = await pushRegistration();
    if (!registration) return { state: "unsupported" };
    const subscription = await registration.pushManager.getSubscription();
    if (!subscription || !subscription.endpoint) {
      return { state: "not-enrolled" };
    }
    let accountMatch = null;
    if (Array.isArray(accountEndpointDigests)) {
      const digest = await endpointDigestHex(subscription.endpoint);
      if (digest) {
        accountMatch = accountEndpointDigests.some(
          (candidate) => typeof candidate === "string" && candidate.toLowerCase() === digest,
        );
      }
    }
    return { state: "enrolled", endpoint: subscription.endpoint, accountMatch };
  } catch (_) {
    return { state: "not-enrolled" };
  }
}

/** Decode an unpadded base64url VAPID public key into the byte array
 * `pushManager.subscribe` expects. */
export function urlBase64ToUint8Array(base64String: unknown): Uint8Array<ArrayBuffer> {
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

function subscriptionKeys(
  subscription: PushSubscription,
): { p256dh: string; auth: string } {
  const json = subscription.toJSON ? subscription.toJSON() : {};
  const keys = json.keys || {};
  if (!keys.p256dh || !keys.auth) {
    throw new Error("push subscription did not expose p256dh/auth keys");
  }
  return { p256dh: keys.p256dh, auth: keys.auth };
}

function notificationRegistrationDigests(
  response: NotificationSetupStatusResponse,
  extensionId: string,
): string[] {
  if (
    response.extension_id !== extensionId ||
    response.requires_setup !== true ||
    typeof response.detail !== "object" ||
    response.detail === null ||
    !("registrations" in response.detail) ||
    !Array.isArray(response.detail.registrations)
  ) {
    throw new Error("backend returned invalid push enrollment evidence");
  }
  const digests = response.detail.registrations.map((registration) => {
    if (
      typeof registration !== "object" ||
      registration === null ||
      !("endpoint_digest" in registration) ||
      typeof registration.endpoint_digest !== "string"
    ) {
      throw new Error("backend returned invalid push enrollment evidence");
    }
    return registration.endpoint_digest.toLowerCase();
  });
  return digests;
}

async function localEndpointDigest(endpoint: string): Promise<string> {
  const digest = await endpointDigestHex(endpoint);
  if (!digest) {
    throw new Error("unable to verify push enrollment evidence");
  }
  return digest;
}

/** Ask for permission, subscribe this browser, and register the
 * subscription with the backend. Returns the resulting browser state.
 *
 * Also the "enable for this account" path when the browser already holds a
 * subscription enrolled by a different account: the existing subscription is
 * reused as-is and registered under the current caller — never unsubscribed,
 * so the other account's enrollment is left intact. */
export async function enrollThisBrowser({
  vapidPublicKey,
}: { vapidPublicKey?: string } = {}): Promise<DevicePushState> {
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
  let createdSubscription = false;
  if (!subscription) {
    subscription = await registration.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: urlBase64ToUint8Array(vapidPublicKey),
    });
    createdSubscription = true;
  }
  try {
    const extensionId = getSessionChannelExtensionId();
    const response = await enableNotificationSetup({
      extensionId,
      payload: {
        endpoint: subscription.endpoint,
        keys: subscriptionKeys(subscription),
        user_agent: typeof navigator !== "undefined" ? navigator.userAgent : undefined,
      },
    });
    const endpointDigest = await localEndpointDigest(subscription.endpoint);
    if (
      !notificationRegistrationDigests(response, extensionId).includes(
        endpointDigest,
      )
    ) {
      throw new Error("backend did not confirm push enrollment");
    }
  } catch (error) {
    // Evidence rule: the browser must never report "enrolled" without a
    // server record. Roll back a subscription THIS call created; a
    // pre-existing one is left alone (it may back another account).
    if (createdSubscription) {
      try {
        await subscription.unsubscribe();
      } catch (rollbackError) {
        console.warn("device push enrollment rollback failed", rollbackError);
      }
    }
    throw error;
  }
  return { state: "enrolled", endpoint: subscription.endpoint, accountMatch: true };
}

/** Unsubscribe this browser locally and remove it from the backend.
 * Returns the resulting browser state.
 *
 * Only call this when the subscription is verified to belong to the current
 * account (`accountMatch === true` from [`getDevicePushState`]): the
 * push subscription is browser-global, so unsubscribing it on behalf of the
 * wrong account would silently break the owning account's notifications. */
export async function unenrollThisBrowser(): Promise<DevicePushState> {
  if (!pushSupported()) return { state: "unsupported" };
  const registration = await pushRegistration();
  if (!registration) return { state: "unsupported" };
  const subscription = await registration.pushManager.getSubscription();
  if (!subscription) return { state: "not-enrolled" };
  const endpoint = subscription.endpoint;
  const extensionId = getSessionChannelExtensionId();
  const response = await disableNotificationSetup({
    extensionId,
    payload: { endpoint },
  });
  const endpointDigest = await localEndpointDigest(endpoint);
  if (
    notificationRegistrationDigests(response, extensionId).includes(
      endpointDigest,
    )
  ) {
    throw new Error("backend did not confirm push removal");
  }
  const unsubscribed = await subscription.unsubscribe();
  if (unsubscribed === false) {
    throw new Error("browser did not confirm push subscription removal");
  }
  return { state: "not-enrolled" };
}
