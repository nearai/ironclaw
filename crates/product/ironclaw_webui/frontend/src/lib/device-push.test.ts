import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { afterEach, test, vi } from "vitest";

vi.mock("./api", () => ({
  enableNotificationSetup: vi.fn(async () => ({ enabled: true })),
  disableNotificationSetup: vi.fn(async () => ({ enabled: false })),
  getSessionChannelExtensionId: vi.fn(() => "session-channel"),
}));

vi.mock("./notification-setup-api", () => ({
  enableNotificationSetup: vi.fn(async () => ({ enabled: true })),
  disableNotificationSetup: vi.fn(async () => ({ enabled: false })),
}));

import {
  disableNotificationSetup,
  enableNotificationSetup,
} from "./notification-setup-api";
import {
  endpointDigestHex,
  enrollThisBrowser,
  getDevicePushState,
  registerServiceWorker,
  unenrollThisBrowser,
  urlBase64ToUint8Array,
} from "./device-push";

const disableNotificationSetupMock = vi.mocked(disableNotificationSetup);
const enableNotificationSetupMock = vi.mocked(enableNotificationSetup);

const originalNavigator = Object.getOwnPropertyDescriptor(globalThis, "navigator");
const originalWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
const originalNotification = Object.getOwnPropertyDescriptor(globalThis, "Notification");

function setGlobal(name, value) {
  Object.defineProperty(globalThis, name, {
    value,
    configurable: true,
    writable: true,
  });
}

function restoreGlobal(name, descriptor) {
  if (descriptor) {
    Object.defineProperty(globalThis, name, descriptor);
  } else {
    delete globalThis[name];
  }
}

afterEach(() => {
  restoreGlobal("navigator", originalNavigator);
  restoreGlobal("window", originalWindow);
  restoreGlobal("Notification", originalNotification);
  vi.clearAllMocks();
});

function sha256Hex(value) {
  return createHash("sha256").update(value).digest("hex");
}

function setupResponse(endpointDigests) {
  return {
    extension_id: "session-channel",
    requires_setup: true,
    enabled: endpointDigests.length > 0,
    detail: {
      registrations: endpointDigests.map((endpoint_digest) => ({ endpoint_digest })),
    },
  };
}

function browserEnvironment({
  permission = "default",
  subscription = null,
  subscribeResult = null,
  hasRegistration = true,
} = {}) {
  const subscribeCalls = [];
  const registration = {
    pushManager: {
      getSubscription: async () => subscription,
      subscribe: async (options) => {
        subscribeCalls.push(options);
        if (subscribeResult) return subscribeResult;
        throw new Error("no subscribe result configured");
      },
    },
  };
  const registerCalls = [];
  setGlobal("navigator", {
    userAgent: "TestBrowser/1.0",
    serviceWorker: {
      register: async (url) => {
        registerCalls.push(url);
        return registration;
      },
      // The module must never await `serviceWorker.ready` — it hangs forever
      // when no registration exists — so the fake only offers the prompt
      // `getRegistration()` probe, and `undefined` models the failed-boot-
      // registration case.
      getRegistration: async () => (hasRegistration ? registration : undefined),
    },
  });
  setGlobal("window", { PushManager: function PushManager() {} });
  setGlobal("Notification", {
    permission,
    requestPermission: async () => {
      const next = permission === "default" ? "granted" : permission;
      Object.defineProperty(globalThis.Notification, "permission", {
        configurable: true,
        value: next,
      });
      return next;
    },
  });
  return { registration, registerCalls, subscribeCalls };
}

function fakeSubscription(endpoint) {
  return {
    endpoint,
    toJSON: () => ({ endpoint, keys: { p256dh: "pk", auth: "as" } }),
    unsubscribe: vi.fn(async () => true),
  };
}

test("urlBase64ToUint8Array decodes an unpadded base64url key", () => {
  // "AQAB" base64url → bytes [1, 0, 1].
  assert.deepEqual(Array.from(urlBase64ToUint8Array("AQAB")), [1, 0, 1]);
  // URL-safe alphabet round-trip: "_-8" → [0xff, 0xef].
  assert.deepEqual(Array.from(urlBase64ToUint8Array("_-8")), [0xff, 0xef]);
  assert.throws(() => urlBase64ToUint8Array(""), /base64url key is required/);
});

test("endpointDigestHex matches an independent SHA-256 and rejects junk", async () => {
  const endpoint = "https://fcm.googleapis.com/fcm/send/abc";
  assert.equal(await endpointDigestHex(endpoint), sha256Hex(endpoint));
  assert.equal(await endpointDigestHex(""), null);
  assert.equal(await endpointDigestHex(undefined), null);
});

test("registerServiceWorker registers /sw.js and swallows failures", async () => {
  const { registerCalls } = browserEnvironment();
  await registerServiceWorker();
  assert.deepEqual(registerCalls, ["/sw.js"]);

  setGlobal("navigator", {
    serviceWorker: {
      register: async () => {
        throw new Error("registration exploded");
      },
    },
  });
  const result = await registerServiceWorker();
  assert.equal(result, null, "a failed registration resolves null, never throws");

  setGlobal("navigator", {});
  assert.equal(await registerServiceWorker(), null, "no serviceWorker support is a no-op");
});

test("getDevicePushState distinguishes unsupported, denied, not-enrolled, and enrolled", async () => {
  setGlobal("navigator", {});
  setGlobal("window", {});
  assert.deepEqual(await getDevicePushState(), { state: "unsupported" });

  browserEnvironment({ permission: "denied" });
  assert.deepEqual(await getDevicePushState(), { state: "permission-denied" });

  browserEnvironment({ permission: "granted", subscription: null });
  assert.deepEqual(await getDevicePushState(), { state: "not-enrolled" });

  browserEnvironment({
    permission: "granted",
    subscription: fakeSubscription("https://fcm.googleapis.com/fcm/send/abc"),
  });
  assert.deepEqual(await getDevicePushState(), {
    state: "enrolled",
    endpoint: "https://fcm.googleapis.com/fcm/send/abc",
    accountMatch: null,
  });
});

test("getDevicePushState resolves promptly as unsupported when no registration exists", async () => {
  // A failed boot registration leaves getRegistration() → undefined;
  // `serviceWorker.ready` would hang forever here, which is the regression
  // this pins (CodeRabbit stability finding on PR #7398).
  browserEnvironment({ permission: "granted", hasRegistration: false });
  assert.deepEqual(await getDevicePushState(), { state: "unsupported" });
});

test("getDevicePushState correlates the subscription with the account's endpoint digests", async () => {
  const endpoint = "https://fcm.googleapis.com/fcm/send/mine";
  browserEnvironment({ permission: "granted", subscription: fakeSubscription(endpoint) });

  const matched = await getDevicePushState({
    accountEndpointDigests: [sha256Hex(endpoint)],
  });
  assert.deepEqual(matched, { state: "enrolled", endpoint, accountMatch: true });

  const matchedCaseInsensitive = await getDevicePushState({
    accountEndpointDigests: [sha256Hex(endpoint).toUpperCase()],
  });
  assert.equal(matchedCaseInsensitive.state, "enrolled");
  assert.equal(matchedCaseInsensitive.accountMatch, true);

  // Another account's browser subscription: digests exist, none match.
  const foreign = await getDevicePushState({
    accountEndpointDigests: [sha256Hex("https://fcm.googleapis.com/fcm/send/other")],
  });
  assert.deepEqual(foreign, { state: "enrolled", endpoint, accountMatch: false });

  const emptyAccount = await getDevicePushState({ accountEndpointDigests: [] });
  assert.equal(emptyAccount.state, "enrolled");
  assert.equal(emptyAccount.accountMatch, false, "an empty digest list is a definite non-match");
});

test("enrollThisBrowser subscribes with the VAPID key and registers with the backend", async () => {
  const endpoint = "https://fcm.googleapis.com/fcm/send/new";
  const { subscribeCalls } = browserEnvironment({
    permission: "default",
    subscription: null,
    subscribeResult: fakeSubscription(endpoint),
  });
  enableNotificationSetupMock.mockResolvedValueOnce(
    setupResponse([sha256Hex(endpoint)]),
  );

  const state = await enrollThisBrowser({ vapidPublicKey: "AQAB" });

  assert.deepEqual(state, {
    state: "enrolled",
    endpoint: "https://fcm.googleapis.com/fcm/send/new",
    accountMatch: true,
  });
  assert.equal(subscribeCalls.length, 1);
  assert.equal(subscribeCalls[0].userVisibleOnly, true);
  assert.deepEqual(Array.from(subscribeCalls[0].applicationServerKey), [1, 0, 1]);
  assert.equal(enableNotificationSetupMock.mock.calls.length, 1);
  assert.deepEqual(enableNotificationSetupMock.mock.calls[0][0], {
    extensionId: "session-channel",
    payload: {
      endpoint: "https://fcm.googleapis.com/fcm/send/new",
      keys: { p256dh: "pk", auth: "as" },
      user_agent: "TestBrowser/1.0",
    },
  });
});

test("enrollThisBrowser rolls back a freshly created subscription when the backend rejects", async () => {
  const created = fakeSubscription("https://fcm.googleapis.com/fcm/send/fresh");
  browserEnvironment({
    permission: "granted",
    subscription: null,
    subscribeResult: created,
  });
  enableNotificationSetupMock.mockRejectedValueOnce(new Error("backend rejected"));

  await assert.rejects(enrollThisBrowser({ vapidPublicKey: "AQAB" }), /backend rejected/);
  assert.equal(
    created.unsubscribe.mock.calls.length,
    1,
    "a created-but-unregistered subscription must be unsubscribed so the browser never reports enrolled without a server record",
  );
});

test("enrollThisBrowser never unsubscribes a pre-existing subscription on backend failure", async () => {
  // The pre-existing subscription may belong to ANOTHER account in this
  // browser profile; rolling it back would sever that account's enrollment.
  const existing = fakeSubscription("https://fcm.googleapis.com/fcm/send/other-account");
  browserEnvironment({ permission: "granted", subscription: existing });
  enableNotificationSetupMock.mockRejectedValueOnce(new Error("backend rejected"));

  await assert.rejects(enrollThisBrowser({ vapidPublicKey: "AQAB" }), /backend rejected/);
  assert.equal(existing.unsubscribe.mock.calls.length, 0);
});

test("enrollThisBrowser rejects a 2xx response without matching backend evidence", async () => {
  const created = fakeSubscription("https://fcm.googleapis.com/fcm/send/unconfirmed");
  browserEnvironment({
    permission: "granted",
    subscription: null,
    subscribeResult: created,
  });
  enableNotificationSetupMock.mockResolvedValueOnce(setupResponse([]));

  await assert.rejects(
    enrollThisBrowser({ vapidPublicKey: "AQAB" }),
    /backend did not confirm push enrollment/,
  );
  assert.equal(created.unsubscribe.mock.calls.length, 1);
});

test("enrollThisBrowser reports a denied permission without subscribing", async () => {
  const { subscribeCalls } = browserEnvironment({ permission: "denied" });
  const state = await enrollThisBrowser({ vapidPublicKey: "AQAB" });
  assert.deepEqual(state, { state: "permission-denied" });
  assert.equal(subscribeCalls.length, 0);
  assert.equal(enableNotificationSetupMock.mock.calls.length, 0);
  await assert.rejects(enrollThisBrowser({}), /vapidPublicKey is required/);
});

test("unenrollThisBrowser preserves the local subscription when backend removal fails", async () => {
  const subscription = fakeSubscription("https://fcm.googleapis.com/fcm/send/old");
  browserEnvironment({ permission: "granted", subscription });
  disableNotificationSetupMock.mockRejectedValueOnce(new Error("backend offline"));

  await assert.rejects(unenrollThisBrowser(), /backend offline/);

  assert.equal(subscription.unsubscribe.mock.calls.length, 0);
  assert.equal(disableNotificationSetupMock.mock.calls.length, 1);
  assert.deepEqual(disableNotificationSetupMock.mock.calls[0][0], {
    extensionId: "session-channel",
    payload: { endpoint: "https://fcm.googleapis.com/fcm/send/old" },
  });
});

test("unenrollThisBrowser requires evidence that the endpoint was removed", async () => {
  const endpoint = "https://fcm.googleapis.com/fcm/send/old";
  const subscription = fakeSubscription(endpoint);
  browserEnvironment({ permission: "granted", subscription });
  disableNotificationSetupMock.mockResolvedValueOnce(
    setupResponse([sha256Hex(endpoint)]),
  );

  await assert.rejects(
    unenrollThisBrowser(),
    /backend did not confirm push removal/,
  );
  assert.equal(subscription.unsubscribe.mock.calls.length, 0);
});

test("unenrollThisBrowser unsubscribes after backend removal is confirmed", async () => {
  const subscription = fakeSubscription("https://fcm.googleapis.com/fcm/send/old");
  browserEnvironment({ permission: "granted", subscription });
  disableNotificationSetupMock.mockResolvedValueOnce(setupResponse([]));

  assert.deepEqual(await unenrollThisBrowser(), { state: "not-enrolled" });
  assert.equal(subscription.unsubscribe.mock.calls.length, 1);
});
