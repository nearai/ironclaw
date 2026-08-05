// @vitest-environment happy-dom

import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { test, vi } from "vitest";
import "../../../i18n/en";
import { I18nProvider } from "../../../lib/i18n";

const requests = vi.hoisted(() => ({
  fetchThreads: vi.fn(),
  fetchArtifact: vi.fn(),
  fetchRunArtifact: vi.fn(),
}));

vi.mock("../lib/admin-api", () => ({
  fetchThreadScrapeThreads: requests.fetchThreads,
  fetchThreadScrapeArtifact: requests.fetchArtifact,
  fetchThreadScrapeRunArtifact: requests.fetchRunArtifact,
}));

vi.mock("../../../lib/download", () => ({ saveBlob: vi.fn() }));

import { ThreadScrapingPanel } from "./thread-scraping-panel";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

test("thread scraping loads every page and ignores stale artifact responses", async () => {
  const firstArtifact = deferred<Record<string, unknown>>();
  const secondArtifact = deferred<Record<string, unknown>>();
  requests.fetchThreads
    .mockResolvedValueOnce({
      threads: [{ thread_id: "thread-one", title: "One" }],
      next_cursor: "cursor-one",
    })
    .mockResolvedValueOnce({
      threads: [
        { thread_id: "thread-one", title: "One duplicate" },
        { thread_id: "thread-two", title: "Two" },
      ],
      next_cursor: null,
    });
  requests.fetchArtifact.mockImplementation((_userId, threadId) =>
    threadId === "thread-one" ? firstArtifact.promise : secondArtifact.promise,
  );

  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => {
      root.render(
        <I18nProvider>
          <ThreadScrapingPanel userId="user-target" />
        </I18nProvider>,
      );
    });

    const loadMore = container.querySelector<HTMLButtonElement>(
      '[data-testid="admin-thread-scraping-load-more"]',
    );
    assert.ok(loadMore, "a next cursor renders the load-more control");
    await act(async () => loadMore.click());

    const secondPageCall = requests.fetchThreads.mock.calls[1];
    assert.equal(secondPageCall?.[0], "user-target");
    assert.equal(secondPageCall?.[1]?.limit, 100);
    assert.equal(secondPageCall?.[1]?.cursor, "cursor-one");
    const threadButtons = Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '[data-testid="admin-thread-scraping-thread"]',
      ),
    );
    assert.equal(threadButtons.length, 2, "pages merge without duplicate thread ids");
    assert.equal(
      container.querySelector('[data-testid="admin-thread-scraping-load-more"]'),
      null,
      "the control disappears when the final page has no cursor",
    );

    await act(async () => {
      threadButtons[0]?.click();
      threadButtons[1]?.click();
    });
    await act(async () => {
      secondArtifact.resolve({
        thread_id: "thread-two",
        messages: [{ message_id: "message-two", kind: "assistant", content: "new selection" }],
      });
    });
    await act(async () => {
      firstArtifact.resolve({
        thread_id: "thread-one",
        messages: [{ message_id: "message-one", kind: "assistant", content: "stale selection" }],
      });
    });

    assert.match(container.textContent ?? "", /new selection/);
    assert.doesNotMatch(container.textContent ?? "", /stale selection/);
  } finally {
    act(() => root.unmount());
    container.remove();
    requests.fetchThreads.mockReset();
    requests.fetchArtifact.mockReset();
    requests.fetchRunArtifact.mockReset();
  }
});
