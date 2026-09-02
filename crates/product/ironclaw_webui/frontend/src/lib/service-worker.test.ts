// Behavior of `public/sw.js` (run-completion push presentation and click
// routing, 2026-08-13 design §9.1–§9.2), driven through the worker's own
// event listeners against a scripted ServiceWorkerGlobalScope. IndexedDB is
// absent here, which exercises the memory ledger; the IndexedDB ledger is
// the same test-and-set contract behind a real browser API.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";

const WORKER_SOURCE = readFileSync(
  new URL("../../public/sw.js", import.meta.url),
  "utf8",
);
const ORIGIN = "https://app.example";

type ShownNotification = { title: string; options: Record<string, unknown> };
type FakeClient = {
  url: string;
  focused: boolean;
  navigatedTo: string | null;
  focus?: () => Promise<FakeClient>;
  navigate?: (url: string) => Promise<FakeClient>;
};

function windowClient(url: string, options: { focusable?: boolean; navigable?: boolean } = {}) {
  const client: FakeClient = { url, focused: false, navigatedTo: null };
  if (options.focusable !== false) {
    client.focus = async () => {
      client.focused = true;
      return client;
    };
  }
  if (options.navigable !== false) {
    client.navigate = async (url: string) => {
      client.navigatedTo = url;
      client.url = new URL(url, ORIGIN).toString();
      return client;
    };
  }
  return client;
}

function loadWorker() {
  const listeners = new Map<string, (event: unknown) => void>();
  const shown: ShownNotification[] = [];
  const opened: string[] = [];
  const clients: FakeClient[] = [];
  const scope = {
    addEventListener: (name: string, handler: (event: unknown) => void) => {
      listeners.set(name, handler);
    },
    skipWaiting: () => undefined,
    clients: {
      claim: async () => undefined,
      matchAll: async () => clients,
      openWindow: async (url: string) => {
        opened.push(url);
        return null;
      },
    },
    registration: {
      showNotification: async (title: string, options: Record<string, unknown>) => {
        shown.push({ title, options });
      },
    },
    location: { origin: ORIGIN },
    indexedDB: undefined,
  };
  // The worker script only reaches the outside world through `self` (and
  // the global URL constructor), so evaluating it against a scripted scope
  // exercises the real handlers.
  new Function("self", WORKER_SOURCE)(scope);

  async function dispatch(name: string, event: Record<string, unknown>) {
    const pending: Promise<unknown>[] = [];
    const handler = listeners.get(name);
    assert.ok(handler, `worker registers a ${name} listener`);
    handler({ ...event, waitUntil: (work: Promise<unknown>) => pending.push(work) });
    await Promise.all(pending);
  }

  return {
    shown,
    opened,
    clients,
    push: (payload: unknown) =>
      dispatch("push", { data: { json: () => payload } }),
    click: (data: unknown) => {
      const closed = { value: false };
      return dispatch("notificationclick", {
        notification: {
          data,
          close: () => {
            closed.value = true;
          },
        },
      }).then(() => closed.value);
    },
  };
}

function runCompletionPayload(overrides: Record<string, unknown> = {}) {
  return {
    schema: "web_app_notification.v2",
    kind: "run_completion",
    title: "IronClaw",
    body: "An agent run finished.",
    url: "/chat/thread-a",
    tag: "rct-thread-a",
    notice_id: "rcn-1",
    ...overrides,
  };
}

test("a run-completion push presents exactly once per notice id", async () => {
  const worker = loadWorker();
  await worker.push(runCompletionPayload());
  await worker.push(runCompletionPayload());
  assert.equal(worker.shown.length, 1, "a re-sent push for the same notice is collapsed");
  assert.equal(worker.shown[0].title, "IronClaw");
  assert.equal(worker.shown[0].options.body, "An agent run finished.");
  assert.equal(worker.shown[0].options.tag, "rct-thread-a");
  assert.deepEqual(worker.shown[0].options.data, { url: "/chat/thread-a" });

  // A different notice for the same thread replaces the OS surface by tag
  // and carries the grouped fixed copy — never generated content.
  await worker.push(runCompletionPayload({ notice_id: "rcn-2", unread_count: 3 }));
  assert.equal(worker.shown.length, 2);
  assert.equal(worker.shown[1].options.body, "3 agent runs finished.");
  assert.equal(worker.shown[1].options.tag, "rct-thread-a");
});

test("push deep links collapse to the app origin and v1 payloads still present", async () => {
  const worker = loadWorker();
  await worker.push(
    runCompletionPayload({ notice_id: "rcn-x", url: "https://evil.example/phish" }),
  );
  assert.deepEqual(worker.shown[0].options.data, { url: "/" });

  await worker.push({ title: "Automation", body: "A routine ran.", url: "/automations" });
  assert.equal(worker.shown.length, 2);
  assert.equal(worker.shown[1].options.body, "A routine ran.");
  assert.deepEqual(worker.shown[1].options.data, { url: "/automations" });

  // Malformed data never throws out of the handler: the fallback copy shows.
  await worker.push(undefined);
  assert.equal(worker.shown[2].options.body, "You have a new notification.");
});

test("notification clicks prefer the client on the path, then a focusable client, then a new window", async () => {
  const worker = loadWorker();
  const elsewhere = windowClient(`${ORIGIN}/automations`);
  const onThread = windowClient(`${ORIGIN}/chat/thread-a`);
  const foreign = windowClient("https://other.example/chat/thread-a");
  worker.clients.push(foreign, elsewhere, onThread);

  // 1. A same-origin client already on the target path just gets focus.
  assert.equal(await worker.click({ url: "/chat/thread-a" }), true, "the notification closes");
  assert.equal(onThread.focused, true);
  assert.equal(elsewhere.focused, false);
  assert.equal(foreign.focused, false, "cross-origin clients are never touched");
  assert.deepEqual(worker.opened, []);

  // 2. No client on the path: the first focusable same-origin client is
  //    navigated there and focused.
  worker.clients.length = 0;
  const reusable = windowClient(`${ORIGIN}/settings`);
  worker.clients.push(foreign, reusable);
  await worker.click({ url: "/chat/thread-b?x=1" });
  assert.equal(reusable.navigatedTo, "/chat/thread-b?x=1");
  assert.equal(reusable.focused, true);
  assert.deepEqual(worker.opened, []);

  // 3. No same-origin window at all: open one — on a same-origin path even
  //    when the stored notification data predates this worker's checks.
  worker.clients.length = 0;
  worker.clients.push(foreign);
  await worker.click({ url: "https://evil.example/anything" });
  assert.deepEqual(worker.opened, ["/"]);
});
