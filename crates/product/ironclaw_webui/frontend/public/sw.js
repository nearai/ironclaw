// IronClaw service worker: Web Push display + notification deep links.
//
// Push payload contracts (`ironclaw_web_app::WebAppNotificationPayload`):
//   v1 — `{ title, body, url, tag? }`;
//   v2 — v1 plus `{ schema: "web_app_notification.v2", kind, notice_id,
//        thread_id, unread_count? }` for typed run-completion pushes
//        (2026-08-13 design §7.10). Payloads carry fixed copy only — never
//        generated or protected content.
//
// Deliberately NO fetch handler and NO caching: the app is served fresh by
// the gateway, and a stale-asset cache is a worse failure mode than a
// network round-trip. Installability no longer requires offline support in
// current browsers. Dependency-free by design (served from public/).

self.addEventListener("install", () => {
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(self.clients.claim());
});

// ---- Run-completion presentation ledger (design §9.2) ----
// IndexedDB test-and-set by notice id collapses duplicate pushes for one
// completion (a re-sent push, or a push racing a live-stream presentation
// recorded by a page). Memory fallback is best-effort when IndexedDB is
// unavailable; the durable server records stay authoritative.

const LEDGER_DB = "ironclaw-run-completions";
const LEDGER_STORE = "presented";
const LEDGER_LIMIT = 250;
const memoryLedger = new Set();

function openLedger() {
  return new Promise((resolve) => {
    if (!self.indexedDB) {
      resolve(null);
      return;
    }
    try {
      // v2 adds the presentedAt index used for oldest-first pruning.
      const request = self.indexedDB.open(LEDGER_DB, 2);
      request.onupgradeneeded = () => {
        const db = request.result;
        const store = db.objectStoreNames.contains(LEDGER_STORE)
          ? request.transaction.objectStore(LEDGER_STORE)
          : db.createObjectStore(LEDGER_STORE, { keyPath: "noticeId" });
        if (!store.indexNames.contains("presentedAt")) {
          store.createIndex("presentedAt", "presentedAt");
        }
      };
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => resolve(null);
      request.onblocked = () => resolve(null);
    } catch (_) {
      resolve(null);
    }
  });
}

// True exactly once per notice id: the first caller claims it.
async function claimNotice(noticeId) {
  if (!noticeId) return true;
  const db = await openLedger();
  if (!db) {
    if (memoryLedger.has(noticeId)) return false;
    memoryLedger.add(noticeId);
    return true;
  }
  return new Promise((resolve) => {
    try {
      const tx = db.transaction(LEDGER_STORE, "readwrite");
      const store = tx.objectStore(LEDGER_STORE);
      const get = store.get(noticeId);
      get.onsuccess = () => {
        if (get.result) {
          resolve(false);
          return;
        }
        store.put({ noticeId, presentedAt: Date.now() });
        pruneLedger(store);
        resolve(true);
      };
      get.onerror = () => resolve(true);
      tx.onabort = () => resolve(true);
    } catch (_) {
      resolve(true);
    } finally {
      // Connections are cheap to reopen and holding one keeps the DB
      // upgrade-blocked for future versions.
      try {
        db.close();
      } catch (_) {
        /* already closed */
      }
    }
  });
}

// Bound the ledger (§5.4: at most 250 active entries; eviction only affects
// dedupe acceleration because the server read state is authoritative).
function pruneLedger(store) {
  try {
    const count = store.count();
    count.onsuccess = () => {
      if (count.result <= LEDGER_LIMIT) return;
      // Oldest-presented first: primary-key order is the opaque noticeId,
      // which would evict arbitrary (possibly recent) presentations.
      const cursor = store.index("presentedAt").openCursor();
      let toDrop = count.result - LEDGER_LIMIT;
      cursor.onsuccess = () => {
        const current = cursor.result;
        if (!current || toDrop <= 0) return;
        current.delete();
        toDrop -= 1;
        current.continue();
      };
    };
  } catch (_) {
    /* best-effort bound */
  }
}

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
    const payload = {
      title: typeof parsed.title === "string" && parsed.title ? parsed.title : fallback.title,
      body: typeof parsed.body === "string" ? parsed.body : fallback.body,
      url:
        typeof parsed.url === "string" && parsed.url
          ? sameOriginPath(parsed.url)
          : fallback.url,
      tag: typeof parsed.tag === "string" && parsed.tag ? parsed.tag : undefined,
    };
    // v2 typed fields (run completions). Unknown schemas keep v1 handling.
    if (parsed.schema === "web_app_notification.v2") {
      payload.kind = typeof parsed.kind === "string" ? parsed.kind : undefined;
      payload.noticeId =
        typeof parsed.notice_id === "string" ? parsed.notice_id : undefined;
      payload.unreadCount =
        typeof parsed.unread_count === "number" && parsed.unread_count > 1
          ? Math.min(parsed.unread_count, 99)
          : undefined;
    }
    return payload;
  } catch (_) {
    return fallback;
  }
}

self.addEventListener("push", (event) => {
  const payload = payloadFromEvent(event);
  event.waitUntil(
    (async () => {
      if (payload.kind === "run_completion") {
        // §9.2 exact dedupe: one presentation per notice id per profile.
        const fresh = await claimNotice(payload.noticeId);
        if (!fresh) return;
        // Same-tag notifications replace each other; grouped copy uses the
        // capped count and generic plural wording (fixed copy only).
        const body =
          payload.unreadCount && payload.unreadCount > 1
            ? `${payload.unreadCount} agent runs finished.`
            : payload.body;
        await self.registration.showNotification(payload.title, {
          body,
          tag: payload.tag,
          data: { url: payload.url },
          icon: "/assets/web-app-manifest-192x192.png",
          badge: "/assets/web-app-manifest-192x192.png",
        });
        return;
      }
      await self.registration.showNotification(payload.title, {
        body: payload.body,
        tag: payload.tag,
        data: { url: payload.url },
        icon: "/assets/web-app-manifest-192x192.png",
        badge: "/assets/web-app-manifest-192x192.png",
      });
    })(),
  );
});

// Click selection (design §9.1): close, validate the same-origin path,
// prefer an existing client already on the target path, then any focusable
// same-origin client (navigating it), and open a new window only when no
// same-origin window exists.
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
        const sameOrigin = clientList.filter((client) => {
          try {
            return new URL(client.url).origin === self.location.origin;
          } catch (_) {
            return false;
          }
        });
        // 1. A client already showing the target path just needs focus.
        for (const client of sameOrigin) {
          try {
            const clientPath = new URL(client.url).pathname;
            if (clientPath === url.split("?")[0].split("#")[0] && "focus" in client) {
              return client.focus();
            }
          } catch (_) {
            /* keep looking */
          }
        }
        // 2. Reuse the first focusable client, navigating it.
        for (const client of sameOrigin) {
          if ("focus" in client) {
            if ("navigate" in client) {
              return client
                .navigate(url)
                .then((navigated) => (navigated || client).focus());
            }
            return client.focus();
          }
        }
        // 3. No same-origin window: open one.
        if (self.clients.openWindow) {
          return self.clients.openWindow(url);
        }
        return undefined;
      }),
  );
});
