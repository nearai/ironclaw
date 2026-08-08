// @ts-nocheck
import assert from "node:assert/strict";
import { afterEach, test, vi } from "vitest";

vi.mock("./api", () => ({
  subscribeWebPush: vi.fn(async () => ({ outcome: "enrolled" })),
  unsubscribeWebPush: vi.fn(async () => ({ removed: true })),
}));

import { subscribeWebPush, unsubscribeWebPush } from "./api";
import {
  enrollThisBrowser,
  getWebPushBrowserState,
  registerServiceWorker,
  unenrollThisBrowser,
  urlBase64ToUint8Array,
} from "./web-push";

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

function browserEnvironment({
  permission = "default",
  subscription = null,
  subscribeResult = null,
  registrationOverrides = {},
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
    ...registrationOverrides,
  };
  const registerCalls = [];
  setGlobal("navigator", {
    userAgent: "TestBrowser/1.0",
    serviceWorker: {
      register: async (url) => {
        registerCalls.push(url);
        return registration;
      },
      ready: Promise.resolve(registration),
    },
  });
  setGlobal("window", { PushManager: function PushManager() {} });
  setGlobal("Notification", {
    permission,
    requestPermission: async () => {
      const next = permission === "default" ? "granted" : permission;
      globalThis.Notification.permission = next;
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

test("getWebPushBrowserState distinguishes unsupported, denied, not-enrolled, and enrolled", async () => {
  setGlobal("navigator", {});
  setGlobal("window", {});
  assert.deepEqual(await getWebPushBrowserState(), { state: "unsupported" });

  browserEnvironment({ permission: "denied" });
  assert.deepEqual(await getWebPushBrowserState(), { state: "permission-denied" });

  browserEnvironment({ permission: "granted", subscription: null });
  assert.deepEqual(await getWebPushBrowserState(), { state: "not-enrolled" });

  browserEnvironment({
    permission: "granted",
    subscription: fakeSubscription("https://fcm.googleapis.com/fcm/send/abc"),
  });
  assert.deepEqual(await getWebPushBrowserState(), {
    state: "enrolled",
    endpoint: "https://fcm.googleapis.com/fcm/send/abc",
  });
});

test("enrollThisBrowser subscribes with the VAPID key and registers with the backend", async () => {
  const { subscribeCalls } = browserEnvironment({
    permission: "default",
    subscription: null,
    subscribeResult: fakeSubscription("https://fcm.googleapis.com/fcm/send/new"),
  });

  const state = await enrollThisBrowser({ vapidPublicKey: "AQAB" });

  assert.deepEqual(state, {
    state: "enrolled",
    endpoint: "https://fcm.googleapis.com/fcm/send/new",
  });
  assert.equal(subscribeCalls.length, 1);
  assert.equal(subscribeCalls[0].userVisibleOnly, true);
  assert.deepEqual(Array.from(subscribeCalls[0].applicationServerKey), [1, 0, 1]);
  assert.equal(subscribeWebPush.mock.calls.length, 1);
  assert.deepEqual(subscribeWebPush.mock.calls[0][0], {
    endpoint: "https://fcm.googleapis.com/fcm/send/new",
    keys: { p256dh: "pk", auth: "as" },
    userAgent: "TestBrowser/1.0",
  });
});

test("enrollThisBrowser reports a denied permission without subscribing", async () => {
  const { subscribeCalls } = browserEnvironment({ permission: "denied" });
  const state = await enrollThisBrowser({ vapidPublicKey: "AQAB" });
  assert.deepEqual(state, { state: "permission-denied" });
  assert.equal(subscribeCalls.length, 0);
  assert.equal(subscribeWebPush.mock.calls.length, 0);
  await assert.rejects(enrollThisBrowser({}), /vapidPublicKey is required/);
});

test("unenrollThisBrowser unsubscribes locally even when the backend removal fails", async () => {
  const subscription = fakeSubscription("https://fcm.googleapis.com/fcm/send/old");
  browserEnvironment({ permission: "granted", subscription });
  unsubscribeWebPush.mockRejectedValueOnce(new Error("backend offline"));

  const state = await unenrollThisBrowser();

  assert.deepEqual(state, { state: "not-enrolled" });
  assert.equal(subscription.unsubscribe.mock.calls.length, 1);
  assert.equal(unsubscribeWebPush.mock.calls.length, 1);
  assert.deepEqual(unsubscribeWebPush.mock.calls[0][0], {
    endpoint: "https://fcm.googleapis.com/fcm/send/old",
  });
});
