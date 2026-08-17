// @vitest-environment happy-dom

import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, test, vi } from "vitest";

vi.mock("react-router", () => ({ useNavigate: () => () => {} }));
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
