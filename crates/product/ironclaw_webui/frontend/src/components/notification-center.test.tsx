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
  await act(async () => {
    container
      .querySelector<HTMLButtonElement>("[data-testid='notification-bell']")
      ?.click();
  });
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
  await act(async () => {
    container
      .querySelector<HTMLButtonElement>("[data-testid='notification-bell']")
      ?.click();
  });
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
  await act(async () => {
    container
      .querySelector<HTMLButtonElement>("[data-testid='notification-bell']")
      ?.click();
  });

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

  const open = async (state) => {
    await act(async () => {
      root.render(<NotificationCenter state={{ messages, unreadIds: new Set(), ...state }} />);
    });
    const bell = container.querySelector<HTMLButtonElement>("[data-testid='notification-bell']");
    if (!document.querySelector("[data-testid='notification-load-more']") && bell) {
      await act(async () => bell.click());
    }
  };

  await open({ canLoadMore: true, loadMore });
  const button = document.querySelector<HTMLButtonElement>(
    "[data-testid='notification-load-more']",
  );
  assert.ok(button, "the control shows while the inbox reports another page");
  await act(async () => button.click());
  assert.equal(loadMore.mock.calls.length, 1);

  await open({ canLoadMore: false, loadMore });
  assert.equal(
    document.querySelector("[data-testid='notification-load-more']"),
    null,
    "the control retires once there is nothing left to page",
  );
});
