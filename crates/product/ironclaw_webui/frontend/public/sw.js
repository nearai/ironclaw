// IronClaw service worker: Web Push display + notification deep links.
//
// The push payload contract is `ironclaw_web_push::WebPushNotificationPayload`
// — `{ title, body, url, tag? }` — encrypted per RFC 8291; the browser hands
// this worker the decrypted JSON. Deliberately NO fetch handler and NO
// caching: the app is served fresh by the gateway, and a stale-asset cache
// is a worse failure mode than a network round-trip. Installability no
// longer requires offline support in current browsers.

self.addEventListener("install", () => {
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(self.clients.claim());
});

// Deep links are same-origin paths by contract. Payloads are produced by our
// own backend, but the notification store outlives deploys and a push
// payload is still external input to this worker — so the contract is
// enforced here, where navigation happens: anything that does not resolve to
// this origin collapses to "/".
function sameOriginPath(value) {
  try {
    const target = new URL(value, self.location.origin);
    if (target.origin !== self.location.origin) return "/";
    return `${target.pathname}${target.search}${target.hash}`;
  } catch (_) {
    return "/";
  }
}

function payloadFromEvent(event) {
  const fallback = {
    title: "IronClaw",
    body: "You have a new notification.",
    url: "/",
  };
  if (!event.data) return fallback;
  try {
    const parsed = event.data.json();
    if (!parsed || typeof parsed !== "object") return fallback;
    return {
      title: typeof parsed.title === "string" && parsed.title ? parsed.title : fallback.title,
      body: typeof parsed.body === "string" ? parsed.body : fallback.body,
      url:
        typeof parsed.url === "string" && parsed.url
          ? sameOriginPath(parsed.url)
          : fallback.url,
      tag: typeof parsed.tag === "string" && parsed.tag ? parsed.tag : undefined,
    };
  } catch (_) {
    return fallback;
  }
}

self.addEventListener("push", (event) => {
  const payload = payloadFromEvent(event);
  event.waitUntil(
    self.registration.showNotification(payload.title, {
      body: payload.body,
      tag: payload.tag,
      data: { url: payload.url },
      icon: "/assets/web-app-manifest-192x192.png",
      badge: "/assets/web-app-manifest-192x192.png",
    }),
  );
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  // Re-validate here as well: stored notifications can predate this worker's
  // version, so `data.url` is not trusted to already be origin-checked.
  const url = sameOriginPath(
    (event.notification.data && event.notification.data.url) || "/",
  );
  event.waitUntil(
    self.clients
      .matchAll({ type: "window", includeUncontrolled: true })
      .then((clientList) => {
        for (const client of clientList) {
          try {
            const clientOrigin = new URL(client.url).origin;
            if (clientOrigin === self.location.origin && "focus" in client) {
              if ("navigate" in client) {
                return client.navigate(url).then((navigated) => (navigated || client).focus());
              }
              return client.focus();
            }
          } catch (_) {
            // Ignore unparseable client URLs and keep looking.
          }
        }
        if (self.clients.openWindow) {
          return self.clients.openWindow(url);
        }
        return undefined;
      }),
  );
});
