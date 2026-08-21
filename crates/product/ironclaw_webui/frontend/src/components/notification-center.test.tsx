// @vitest-environment happy-dom

import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, test, vi } from "vitest";

const { navigate } = vi.hoisted(() => ({ navigate: vi.fn() }));
vi.mock("react-router", () => ({ useNavigate: () => navigate }));
vi.mock("../lib/i18n", () => ({ useT: () => (key) => key }));

import { NotificationCenter } from "./notification-center";

const roots = [];

/* The panel is `React.lazy`, so opening it suspends for a microtask before its
 * markup exists. Every test that asserts on panel contents goes through here. */
async function openPanel(container: HTMLElement) {
  await act(async () => {
    container
      .querySelector<HTMLButtonElement>("[data-testid='notification-bell']")
      ?.click();
  });
  // The dynamic import resolves over several turns of the microtask queue, so
  // settle until the panel is actually mounted rather than guessing a count.
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (document.querySelector("[data-testid='notification-panel']")) return;
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  }
  assert.fail("the lazy notification panel did not mount");
}

afterEach(() => {
  for (const root of roots.splice(0)) {
    act(() => root.unmount());
  }
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

test("notification center renders loading and retryable error states", async () => {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  roots.push(root);

  await act(async () => {
    root.render(<NotificationCenter state={{ messages: [], isLoading: true }} />);
  });
  await openPanel(container);
  assert.equal(document.querySelector("[role='status']")?.textContent,
    "notifications.loadingTitle");

  const refetch = vi.fn();
  await act(async () => {
    root.render(
      <NotificationCenter
        state={{ messages: [], error: new Error("offline"), refetch }}
      />,
    );
  });
  assert.match(
    document.querySelector("[role='alert']")?.textContent || "",
    /notifications.errorTitle/,
  );
  await act(async () => {
    [...document.querySelectorAll("button")]
      .find((button) => button.textContent === "notifications.retry")
      ?.click();
  });
  assert.equal(refetch.mock.calls.length, 1);
});

test("opening a row delegates acknowledgement policy before navigation", async () => {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  roots.push(root);
  const prepareMessageOpen = vi.fn();
  const message = {
    id: "notification-completed",
    title: "Completed",
    body: "Finished",
    href: "/chat/thread-1",
    type: "run_completed",
  };

  await act(async () => {
    root.render(
      <NotificationCenter
        state={{
          messages: [message],
          unreadIds: new Set([message.id]),
          prepareMessageOpen,
        }}
      />,
    );
  });
  await openPanel(container);
  await act(async () => {
    document
      .querySelector<HTMLButtonElement>("[data-testid='notification-row']")
      ?.click();
  });

  assert.equal(prepareMessageOpen.mock.calls[0]?.[0], message);
  assert.deepEqual(navigate.mock.calls[0], ["/chat/thread-1"]);
});

test("archive is offered for durable rows only and does not open the thread", async () => {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  roots.push(root);

  const archiveMessage = vi.fn();
  const prepareMessageOpen = vi.fn();
  const messages = [
    {
      id: "notification-1",
      title: "Approval required",
      body: "A run is waiting",
      href: "/chat/thread-1",
      durable: true,
    },
    {
      id: "approval:thread-legacy",
      title: "Legacy approval",
      body: "From the compatibility read",
      href: "/chat/thread-legacy",
      durable: false,
    },
  ];

  await act(async () => {
    root.render(
      <NotificationCenter
        state={{ messages, archiveMessage, prepareMessageOpen, unreadIds: new Set() }}
      />,
    );
  });
  await openPanel(container);

  const archiveButtons = [
    ...document.querySelectorAll<HTMLButtonElement>("[data-testid='notification-archive']"),
  ];
  assert.equal(
    archiveButtons.length,
    1,
    "only the durable row has a server-side record to archive",
  );

  await act(async () => archiveButtons[0].click());
  assert.deepEqual(archiveMessage.mock.calls, [["notification-1"]]);
  assert.equal(
    prepareMessageOpen.mock.calls.length,
    0,
    "archiving must not also navigate into the thread",
  );
});

test("load-more appears only while more pages remain", async () => {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  roots.push(root);

  const loadMore = vi.fn();
  const messages = [{ id: "n-1", title: "One", body: "b", href: "/chat/t-1", durable: true }];

  /* Render only — the bell is a toggle, so clicking it again to "make sure the
   * panel is open" is what shuts it. Open once, below, and then re-render. */
  const show = async (state) => {
    await act(async () => {
      root.render(<NotificationCenter state={{ messages, unreadIds: new Set(), ...state }} />);
    });
  };

  await show({ canLoadMore: true, loadMore });
  await openPanel(container);
  const button = document.querySelector<HTMLButtonElement>(
    "[data-testid='notification-load-more']",
  );
  assert.ok(button, "the control shows while the inbox reports another page");
  await act(async () => button.click());
  assert.equal(loadMore.mock.calls.length, 1);

  await show({ canLoadMore: false, loadMore });
  assert.ok(
    document.querySelector("[data-testid='notification-panel']"),
    "the panel is still open, so the next assertion is about the control",
  );
  assert.equal(
    document.querySelector("[data-testid='notification-load-more']"),
    null,
    "the control retires once there is nothing left to page",
  );
});

test("a cold-loaded panel takes focus off the bell", async () => {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  roots.push(root);

  await act(async () => {
    root.render(
      <NotificationCenter
        state={{ messages: [], unreadIds: new Set(), isLoading: false }}
      />,
    );
  });
  await openPanel(container);

  const panel = document.querySelector("[data-testid='notification-panel']");
  assert.ok(panel, "the panel mounted");
  // The opener cannot focus it: its effect runs while Suspense still renders
  // null, so the panel has to claim focus when it mounts instead.
  assert.equal(
    document.activeElement,
    panel,
    "focus moves to the panel after the lazy chunk resolves, not back to the bell",
  );
  assert.equal(
    panel?.getAttribute("aria-modal"),
    "true",
    "a portalled dialog over the page announces itself as modal",
  );
});

test("Tab and Shift+Tab stay inside the notification dialog", async () => {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  roots.push(root);

  await act(async () => {
    root.render(
      <NotificationCenter
        state={{
          messages: [{
            id: "notification-1",
            title: "Finished",
            body: "The run completed",
            href: "/chat/thread-1",
            durable: true,
          }],
          unreadIds: new Set(["notification-1"]),
          unreadCount: 1,
          markAllRead: vi.fn(),
          archiveMessage: vi.fn(),
          canLoadMore: true,
          loadMore: vi.fn(),
        }}
      />,
    );
  });
  await openPanel(container);

  const panel = document.querySelector<HTMLElement>("[data-testid='notification-panel']");
  assert.ok(panel);
  const controls = [...panel.querySelectorAll<HTMLElement>(
    "button:not([disabled]):not([tabindex='-1'])",
  )];
  assert.ok(controls.length > 1, "the fixture exposes both ends of the focus ring");
  const first = controls[0];
  const last = controls.at(-1);
  assert.ok(last);

  await act(async () => {
    panel.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true }));
  });
  assert.equal(document.activeElement, first, "Tab from the dialog enters its first control");

  last.focus();
  await act(async () => {
    last.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true }));
  });
  assert.equal(document.activeElement, first, "Tab wraps from the last control");

  first.focus();
  await act(async () => {
    first.dispatchEvent(new KeyboardEvent("keydown", {
      key: "Tab",
      shiftKey: true,
      bubbles: true,
    }));
  });
  assert.equal(document.activeElement, last, "Shift+Tab wraps from the first control");
});

test("a failure while rows are on screen stays visible", async () => {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  roots.push(root);

  const refetch = vi.fn();
  const messages = [{
    id: "notification-1",
    type: "run_completed",
    title: "Run finished",
    href: "/chat/thread-1",
    timestamp: 2,
    read: false,
    durable: true,
  }];

  await act(async () => {
    root.render(<NotificationCenter state={{ messages, refetch }} />);
  });
  await openPanel(container);
  assert.equal(
    document.querySelector("[data-testid='notification-error-banner']"),
    null,
    "a healthy list shows no banner",
  );

  await act(async () => {
    root.render(
      <NotificationCenter
        state={{ messages, error: new Error("mark read failed"), refetch }}
      />,
    );
  });

  const banner = document.querySelector("[data-testid='notification-error-banner']");
  assert.ok(banner, "the failure is reported even though the list still has rows");
  assert.equal(banner?.getAttribute("role"), "alert");
  assert.ok(
    document.querySelectorAll("[data-testid='notification-row']").length > 0,
    "the rows stay on screen alongside the banner",
  );

  await act(async () => {
    [...(banner?.querySelectorAll("button") || [])]
      .find((button) => button.textContent === "notifications.retry")
      ?.click();
  });
  assert.equal(refetch.mock.calls.length, 1);
});

test("shutting the panel tells the reader to stop paging", async () => {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  roots.push(root);

  const collapsePages = vi.fn();
  const messages = [{
    id: "n-1", title: "One", href: "/chat/t-1", timestamp: 1, durable: true,
  }];
  await act(async () => {
    root.render(
      <NotificationCenter
        state={{ messages, unreadIds: new Set(), collapsePages }}
      />,
    );
  });
  await openPanel(container);
  assert.equal(collapsePages.mock.calls.length, 0, "opening does not collapse");

  const closeButton = [...document.querySelectorAll("button")].find(
    (button) => button.getAttribute("aria-label") === "notifications.close",
  );
  await act(async () => closeButton?.click());

  assert.equal(
    document.querySelector("[data-testid='notification-panel']"),
    null,
    "the panel really closed",
  );
  assert.equal(
    collapsePages.mock.calls.length,
    1,
    "a closed panel must not leave the poll walking every loaded page",
  );
});

test("toggling the bell shut also collapses paging", async () => {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  roots.push(root);

  const collapsePages = vi.fn();
  await act(async () => {
    root.render(
      <NotificationCenter state={{ messages: [], unreadIds: new Set(), collapsePages }} />,
    );
  });
  await openPanel(container);

  const bell = container.querySelector<HTMLButtonElement>(
    "[data-testid='notification-bell']",
  );
  await act(async () => bell?.click());

  assert.equal(document.querySelector("[data-testid='notification-panel']"), null);
  assert.equal(collapsePages.mock.calls.length, 1);
});

test("the page-limit notice stands in for a control that cannot retire", async () => {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  roots.push(root);

  const messages = [{
    id: "n-1", title: "One", href: "/chat/t-1", timestamp: 1, durable: true,
  }];
  await act(async () => {
    root.render(
      <NotificationCenter
        state={{ messages, unreadIds: new Set(), canLoadMore: false, pageLimitReached: true }}
      />,
    );
  });
  await openPanel(container);

  const notice = document.querySelector("[data-testid='notification-page-limit']");
  assert.ok(notice, "records past the ceiling are announced, not silently dropped");
  assert.match(notice?.textContent || "", /notifications.pageLimit/);
  assert.equal(
    document.querySelector("[data-testid='notification-load-more']"),
    null,
    "the control is gone, which is exactly why the notice has to be there",
  );
});

test("mark all read is offered only while something is unread", async () => {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  roots.push(root);

  const markAllRead = vi.fn();
  const messages = [{
    id: "n-1", title: "One", href: "/chat/t-1", timestamp: 1, durable: true,
  }];
  const selector = "[data-testid='notification-mark-all-read']";

  await act(async () => {
    root.render(
      <NotificationCenter
        state={{ messages, unreadIds: new Set(["n-1"]), unreadCount: 1, markAllRead }}
      />,
    );
  });
  await openPanel(container);
  const button = document.querySelector<HTMLButtonElement>(selector);
  assert.ok(button, "an unread row offers the control");
  assert.equal(button?.disabled, false);
  await act(async () => button?.click());
  assert.equal(markAllRead.mock.calls.length, 1);

  await act(async () => {
    root.render(
      <NotificationCenter
        state={{ messages, unreadIds: new Set(), unreadCount: 0, markAllRead }}
      />,
    );
  });
  assert.equal(
    document.querySelector(selector),
    null,
    "with nothing unread there is nothing to mark",
  );

  await act(async () => {
    root.render(
      <NotificationCenter
        state={{
          messages,
          unreadIds: new Set(["n-1"]),
          unreadCount: 1,
          markAllRead,
          isMarkingAllRead: true,
        }}
      />,
    );
  });
  assert.equal(
    document.querySelector<HTMLButtonElement>(selector)?.disabled,
    true,
    "an in-flight request must not be fired twice",
  );
});
