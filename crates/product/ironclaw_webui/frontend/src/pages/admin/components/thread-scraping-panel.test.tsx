// @vitest-environment happy-dom

import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider, notifyManager } from "@tanstack/react-query";
import { test, vi } from "vitest";
import "../../../i18n/de";
import "../../../i18n/en";
import { I18nProvider, useI18n } from "../../../lib/i18n";

// react-query schedules observer notifications through setTimeout(0) by
// default; a macrotask is never flushed by `await act(...)`, so run
// notifications synchronously and let act's microtask flush settle queries.
notifyManager.setScheduler((callback) => callback());

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

import { saveBlob } from "../../../lib/download";
import { ThreadScrapingPanel } from "./thread-scraping-panel";

function LanguageSwitchingPanel() {
  const { setLang } = useI18n();
  return (
    <>
      <button type="button" data-testid="switch-language" onClick={() => setLang("de")}>
        Deutsch
      </button>
      <ThreadScrapingPanel userId="user-target" />
    </>
  );
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, reject, resolve };
}

function createTestRoot(container, queryClient) {
  const root = createRoot(container);
  return {
    render(node) {
      root.render(
        <QueryClientProvider client={queryClient}>{node}</QueryClientProvider>,
      );
    },
    unmount() {
      root.unmount();
    },
  };
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
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const root = createTestRoot(container, queryClient);
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

test("switching target users discards the previous user's pending artifact", async () => {
  const pendingArtifact = deferred<Record<string, unknown>>();
  requests.fetchThreads
    .mockResolvedValueOnce({
      threads: [{ thread_id: "thread-one", title: "One" }],
      next_cursor: null,
    })
    .mockResolvedValueOnce({ threads: [], next_cursor: null });
  requests.fetchArtifact.mockReturnValue(pendingArtifact.promise);

  const container = document.createElement("div");
  document.body.append(container);
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const root = createTestRoot(container, queryClient);
  try {
    await act(async () => {
      root.render(
        <I18nProvider>
          <ThreadScrapingPanel userId="user-one" />
        </I18nProvider>,
      );
    });
    const threadButton = container.querySelector<HTMLButtonElement>(
      '[data-testid="admin-thread-scraping-thread"]',
    );
    assert.ok(threadButton, "the first user's thread renders");
    await act(async () => threadButton.click());
    assert.deepEqual(
      requests.fetchArtifact.mock.calls.map((call) => call.slice(0, 2)),
      [["user-one", "thread-one"]],
    );

    await act(async () => {
      root.render(
        <I18nProvider>
          <ThreadScrapingPanel userId="user-two" />
        </I18nProvider>,
      );
    });
    assert.equal(
      container.querySelector('[data-testid="admin-thread-scraping-thread"]'),
      null,
      "changing targets clears the previous user's thread selection",
    );

    await act(async () => {
      pendingArtifact.resolve({
        thread_id: "thread-one",
        messages: [
          {
            message_id: "message-one",
            kind: "assistant",
            content: "previous user transcript",
          },
        ],
      });
    });

    assert.doesNotMatch(container.textContent ?? "", /previous user transcript/);
    assert.deepEqual(
      requests.fetchThreads.mock.calls.map((call) => call[0]),
      ["user-one", "user-two"],
    );
  } finally {
    act(() => root.unmount());
    container.remove();
    requests.fetchThreads.mockReset();
    requests.fetchArtifact.mockReset();
    requests.fetchRunArtifact.mockReset();
  }
});

test("run artifact requests abort when the target user or thread changes", async () => {
  const pendingRun = deferred<Record<string, unknown>>();
  requests.fetchThreads.mockImplementation((userId) =>
    Promise.resolve({
      threads: [
        {
          thread_id: userId === "user-one" ? "thread-one" : "thread-two",
          title: userId === "user-one" ? "One" : "Two",
        },
      ],
      next_cursor: null,
    }),
  );
  requests.fetchArtifact.mockImplementation((_userId, threadId) =>
    Promise.resolve({
      thread_id: threadId,
      messages: [
        {
          message_id: `message-${threadId}`,
          kind: "assistant",
          run_id: threadId === "thread-one" ? "run-one" : "run-two",
          content: threadId,
        },
      ],
    }),
  );
  requests.fetchRunArtifact.mockReturnValue(pendingRun.promise);

  const container = document.createElement("div");
  document.body.append(container);
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const root = createTestRoot(container, queryClient);
  try {
    await act(async () => {
      root.render(
        <I18nProvider>
          <ThreadScrapingPanel userId="user-one" />
        </I18nProvider>,
      );
    });
    await act(async () => {
      container
        .querySelector<HTMLButtonElement>('[data-testid="admin-thread-scraping-thread"]')
        ?.click();
    });
    const firstRunButton = Array.from(container.querySelectorAll<HTMLButtonElement>("button")).find(
      (button) => button.textContent?.includes("run-one"),
    );
    assert.ok(firstRunButton);
    await act(async () => firstRunButton.click());
    const firstRunSignal = requests.fetchRunArtifact.mock.calls[0]?.[3]?.signal;
    assert.ok(firstRunSignal, "the run request carries a dedicated abort signal");

    // Selecting another thread cancels the in-flight run request.
    await act(async () => {
      container
        .querySelector<HTMLButtonElement>('[data-testid="admin-thread-scraping-thread"]')
        ?.click();
    });
    assert.equal(firstRunSignal.aborted, true);
    assert.equal(vi.mocked(saveBlob).mock.calls.length, 0);

    await act(async () => {
      root.render(
        <I18nProvider>
          <ThreadScrapingPanel userId="user-two" />
        </I18nProvider>,
      );
    });
    await act(async () => {
      container
        .querySelector<HTMLButtonElement>('[data-testid="admin-thread-scraping-thread"]')
        ?.click();
    });
    const secondRunButton = Array.from(container.querySelectorAll<HTMLButtonElement>("button")).find(
      (button) => button.textContent?.includes("run-two"),
    );
    assert.ok(secondRunButton);
    await act(async () => secondRunButton.click());
    const secondRunSignal = requests.fetchRunArtifact.mock.calls[1]?.[3]?.signal;
    assert.ok(secondRunSignal);
    await act(async () => {
      root.render(
        <I18nProvider>
          <ThreadScrapingPanel userId="user-three" />
        </I18nProvider>,
      );
    });
    assert.equal(secondRunSignal.aborted, true);
    assert.equal(vi.mocked(saveBlob).mock.calls.length, 0);
  } finally {
    act(() => root.unmount());
    container.remove();
    requests.fetchThreads.mockReset();
    requests.fetchArtifact.mockReset();
    requests.fetchRunArtifact.mockReset();
    vi.mocked(saveBlob).mockReset();
  }
});

test("switching target users discards a pending run download", async () => {
  const pendingRun = deferred<Record<string, unknown>>();
  requests.fetchThreads.mockImplementation((userId) =>
    Promise.resolve({
      threads: [
        {
          thread_id: userId === "user-one" ? "thread-one" : "thread-two",
          title: userId === "user-one" ? "One" : "Two",
        },
      ],
      next_cursor: null,
    }),
  );
  requests.fetchArtifact.mockImplementation((_userId, threadId) =>
    Promise.resolve({
      thread_id: threadId,
      messages: [
        {
          message_id: `message-${threadId}`,
          kind: "assistant",
          run_id: threadId === "thread-one" ? "run-one" : "run-two",
          content: threadId,
        },
      ],
    }),
  );
  requests.fetchRunArtifact.mockReturnValue(pendingRun.promise);

  const container = document.createElement("div");
  document.body.append(container);
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const root = createTestRoot(container, queryClient);
  try {
    await act(async () => {
      root.render(
        <I18nProvider>
          <ThreadScrapingPanel userId="user-one" />
        </I18nProvider>,
      );
    });
    await act(async () => {
      container
        .querySelector<HTMLButtonElement>('[data-testid="admin-thread-scraping-thread"]')
        ?.click();
    });
    const firstRunButton = Array.from(container.querySelectorAll<HTMLButtonElement>("button")).find(
      (button) => button.textContent?.includes("run-one"),
    );
    assert.ok(firstRunButton);
    await act(async () => firstRunButton.click());

    await act(async () => {
      root.render(
        <I18nProvider>
          <ThreadScrapingPanel userId="user-two" />
        </I18nProvider>,
      );
    });
    await act(async () => {
      container
        .querySelector<HTMLButtonElement>('[data-testid="admin-thread-scraping-thread"]')
        ?.click();
    });
    const secondRunButton = Array.from(container.querySelectorAll<HTMLButtonElement>("button")).find(
      (button) => button.textContent?.includes("run-two"),
    );
    assert.ok(secondRunButton);
    assert.equal(secondRunButton.disabled, false);

    await act(async () => {
      pendingRun.resolve({ schema: "stale-run-artifact" });
    });
    assert.equal(vi.mocked(saveBlob).mock.calls.length, 0);
  } finally {
    act(() => root.unmount());
    container.remove();
    requests.fetchThreads.mockReset();
    requests.fetchArtifact.mockReset();
    requests.fetchRunArtifact.mockReset();
    vi.mocked(saveBlob).mockReset();
  }
});

test("switching target users discards late initial list responses and errors", async () => {
  const lateResponse = deferred<Record<string, unknown>>();
  const lateError = deferred<Record<string, unknown>>();
  requests.fetchThreads.mockImplementation((userId) => {
    if (userId === "user-one") return lateResponse.promise;
    if (userId === "user-three") return lateError.promise;
    return Promise.resolve({ threads: [], next_cursor: null });
  });

  const container = document.createElement("div");
  document.body.append(container);
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const root = createTestRoot(container, queryClient);
  try {
    await act(async () => {
      root.render(
        <I18nProvider>
          <ThreadScrapingPanel userId="user-one" />
        </I18nProvider>,
      );
    });
    const firstSignal = requests.fetchThreads.mock.calls[0]?.[1]?.signal;
    await act(async () => {
      root.render(
        <I18nProvider>
          <ThreadScrapingPanel userId="user-two" />
        </I18nProvider>,
      );
    });
    assert.equal(firstSignal?.aborted, true);
    await act(async () => {
      lateResponse.resolve({
        threads: [{ thread_id: "stale-thread", title: "Stale initial response" }],
        next_cursor: "stale-cursor",
      });
    });
    assert.doesNotMatch(container.textContent ?? "", /Stale initial response/);

    await act(async () => {
      root.render(
        <I18nProvider>
          <ThreadScrapingPanel userId="user-three" />
        </I18nProvider>,
      );
    });
    const thirdSignal = requests.fetchThreads.mock.calls[2]?.[1]?.signal;
    await act(async () => {
      root.render(
        <I18nProvider>
          <ThreadScrapingPanel userId="user-four" />
        </I18nProvider>,
      );
    });
    assert.equal(thirdSignal?.aborted, true);
    await act(async () => {
      lateError.reject(new Error("stale initial error"));
    });
    assert.equal(container.querySelector('[role="alert"]'), null);
  } finally {
    act(() => root.unmount());
    container.remove();
    requests.fetchThreads.mockReset();
    requests.fetchArtifact.mockReset();
    requests.fetchRunArtifact.mockReset();
  }
});

test("switching target users discards late paginated responses and errors", async () => {
  const latePage = deferred<Record<string, unknown>>();
  const latePageError = deferred<Record<string, unknown>>();
  requests.fetchThreads.mockImplementation((userId, options) => {
    if (options?.cursor === "cursor-one") return latePage.promise;
    if (options?.cursor === "cursor-three") return latePageError.promise;
    if (userId === "user-one") {
      return Promise.resolve({
        threads: [{ thread_id: "thread-one", title: "One" }],
        next_cursor: "cursor-one",
      });
    }
    if (userId === "user-three") {
      return Promise.resolve({
        threads: [{ thread_id: "thread-three", title: "Three" }],
        next_cursor: "cursor-three",
      });
    }
    return Promise.resolve({ threads: [], next_cursor: null });
  });

  const container = document.createElement("div");
  document.body.append(container);
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const root = createTestRoot(container, queryClient);
  try {
    await act(async () => {
      root.render(
        <I18nProvider>
          <ThreadScrapingPanel userId="user-one" />
        </I18nProvider>,
      );
    });
    const firstLoadMore = container.querySelector<HTMLButtonElement>(
      '[data-testid="admin-thread-scraping-load-more"]',
    );
    assert.ok(firstLoadMore);
    await act(async () => firstLoadMore.click());
    const firstPageSignal = requests.fetchThreads.mock.calls[1]?.[1]?.signal;
    await act(async () => {
      root.render(
        <I18nProvider>
          <ThreadScrapingPanel userId="user-two" />
        </I18nProvider>,
      );
    });
    assert.equal(firstPageSignal?.aborted, true);
    await act(async () => {
      latePage.resolve({
        threads: [{ thread_id: "stale-page-thread", title: "Stale page response" }],
        next_cursor: null,
      });
    });
    assert.doesNotMatch(container.textContent ?? "", /Stale page response/);

    await act(async () => {
      root.render(
        <I18nProvider>
          <ThreadScrapingPanel userId="user-three" />
        </I18nProvider>,
      );
    });
    const thirdLoadMore = container.querySelector<HTMLButtonElement>(
      '[data-testid="admin-thread-scraping-load-more"]',
    );
    assert.ok(thirdLoadMore);
    await act(async () => thirdLoadMore.click());
    const thirdPageSignal = requests.fetchThreads.mock.calls[4]?.[1]?.signal;
    await act(async () => {
      root.render(
        <I18nProvider>
          <ThreadScrapingPanel userId="user-four" />
        </I18nProvider>,
      );
    });
    assert.equal(thirdPageSignal?.aborted, true);
    await act(async () => {
      latePageError.reject(new Error("stale paginated error"));
    });
    assert.equal(container.querySelector('[role="alert"]'), null);
  } finally {
    act(() => root.unmount());
    container.remove();
    requests.fetchThreads.mockReset();
    requests.fetchArtifact.mockReset();
    requests.fetchRunArtifact.mockReset();
  }
});

test("thread scraping never renders raw request errors", async () => {
  requests.fetchThreads
    .mockRejectedValueOnce(new Error("sensitive initial details"))
    .mockResolvedValueOnce({
      threads: [{ thread_id: "thread-one", title: "One" }],
      next_cursor: "cursor-one",
    })
    .mockRejectedValueOnce(new Error("sensitive page details"));
  requests.fetchArtifact
    .mockRejectedValueOnce(new Error("sensitive artifact details"))
    .mockResolvedValueOnce({
      thread_id: "thread-one",
      messages: [
        {
          message_id: "message-one",
          run_id: "run-one",
          kind: "assistant",
          content: "safe transcript",
        },
      ],
    });
  requests.fetchRunArtifact.mockRejectedValueOnce(new Error("sensitive download details"));

  const container = document.createElement("div");
  document.body.append(container);
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const root = createTestRoot(container, queryClient);
  try {
    await act(async () => {
      root.render(
        <I18nProvider>
          <ThreadScrapingPanel userId="user-one" />
        </I18nProvider>,
      );
    });
    let alert = container.querySelector('[role="alert"]');
    assert.equal(alert?.textContent, "Thread scraping failed.");
    assert.doesNotMatch(container.textContent ?? "", /sensitive initial details/);
    assert.doesNotMatch(
      container.textContent ?? "",
      /No threads available for scraping/,
      "a failed list request must not render the empty state",
    );

    await act(async () => {
      root.render(
        <I18nProvider>
          <ThreadScrapingPanel userId="user-two" />
        </I18nProvider>,
      );
    });
    const loadMore = container.querySelector<HTMLButtonElement>(
      '[data-testid="admin-thread-scraping-load-more"]',
    );
    assert.ok(loadMore);
    await act(async () => loadMore.click());
    alert = container.querySelector('[role="alert"]');
    assert.equal(alert?.textContent, "Thread scraping failed.");
    assert.doesNotMatch(container.textContent ?? "", /sensitive page details/);

    const threadButton = container.querySelector<HTMLButtonElement>(
      '[data-testid="admin-thread-scraping-thread"]',
    );
    assert.ok(threadButton);
    await act(async () => threadButton.click());
    alert = container.querySelector('[role="alert"]');
    assert.equal(alert?.textContent, "Thread scraping failed.");
    assert.doesNotMatch(container.textContent ?? "", /sensitive artifact details/);

    await act(async () => threadButton.click());
    const downloadRun = Array.from(container.querySelectorAll<HTMLButtonElement>("button")).find(
      (button) => button.textContent?.includes("Download run run-one"),
    );
    assert.ok(downloadRun);
    await act(async () => downloadRun.click());
    alert = container.querySelector('[role="alert"]');
    assert.equal(alert?.textContent, "Artifact download failed.");
    assert.doesNotMatch(container.textContent ?? "", /sensitive download details/);
  } finally {
    act(() => root.unmount());
    container.remove();
    requests.fetchThreads.mockReset();
    requests.fetchArtifact.mockReset();
    requests.fetchRunArtifact.mockReset();
  }
});

test("pending errors use the active locale when they settle", async () => {
  const pendingThreads = deferred<Record<string, unknown>>();
  requests.fetchThreads.mockReturnValue(pendingThreads.promise);

  const container = document.createElement("div");
  document.body.append(container);
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const root = createTestRoot(container, queryClient);
  try {
    await act(async () => {
      root.render(
        <I18nProvider>
          <LanguageSwitchingPanel />
        </I18nProvider>,
      );
    });
    const languageButton = container.querySelector<HTMLButtonElement>('[data-testid="switch-language"]');
    assert.ok(languageButton);
    await act(async () => languageButton.click());
    await act(async () => pendingThreads.reject(new Error("sensitive delayed details")));

    const alert = container.querySelector('[role="alert"]');
    assert.equal(alert?.textContent, "Thread-Scraping fehlgeschlagen.");
    assert.doesNotMatch(container.textContent ?? "", /sensitive delayed details/);
    assert.equal(requests.fetchThreads.mock.calls.length, 1);
  } finally {
    act(() => root.unmount());
    container.remove();
    requests.fetchThreads.mockReset();
    requests.fetchArtifact.mockReset();
    requests.fetchRunArtifact.mockReset();
  }
});

test("switching locales preserves the loaded thread and artifact", async () => {
  requests.fetchThreads.mockResolvedValue({
    threads: [{ thread_id: "thread-one", title: "One" }],
    next_cursor: null,
  });
  requests.fetchArtifact.mockResolvedValue({
    thread_id: "thread-one",
    messages: [{ message_id: "message-one", kind: "assistant", content: "loaded transcript" }],
  });

  const container = document.createElement("div");
  document.body.append(container);
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const root = createTestRoot(container, queryClient);
  try {
    await act(async () => {
      root.render(
        <I18nProvider>
          <LanguageSwitchingPanel />
        </I18nProvider>,
      );
    });
    const threadButton = container.querySelector<HTMLButtonElement>(
      '[data-testid="admin-thread-scraping-thread"]',
    );
    assert.ok(threadButton);
    await act(async () => threadButton.click());
    assert.match(container.textContent ?? "", /loaded transcript/);

    const languageButton = container.querySelector<HTMLButtonElement>('[data-testid="switch-language"]');
    assert.ok(languageButton);
    await act(async () => languageButton.click());

    assert.match(container.textContent ?? "", /Thread-Scraping/);
    assert.match(container.textContent ?? "", /loaded transcript/);
    assert.equal(requests.fetchThreads.mock.calls.length, 1);
  } finally {
    act(() => root.unmount());
    container.remove();
    requests.fetchThreads.mockReset();
    requests.fetchArtifact.mockReset();
    requests.fetchRunArtifact.mockReset();
  }
});

test("thread transcripts render in a bounded window with a show-more expander", async () => {
  const manyMessages = Array.from({ length: 60 }, (_, index) => ({
    message_id: `message-${index}`,
    kind: index % 2 === 0 ? "user" : "assistant",
    content: `content-${index}`,
  }));
  requests.fetchThreads.mockResolvedValueOnce({
    threads: [{ thread_id: "thread-one", title: "One" }],
    next_cursor: null,
  });
  requests.fetchArtifact.mockResolvedValueOnce({
    thread_id: "thread-one",
    messages: manyMessages,
  });

  const container = document.createElement("div");
  document.body.append(container);
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const root = createTestRoot(container, queryClient);
  try {
    await act(async () => {
      root.render(
        <I18nProvider>
          <ThreadScrapingPanel userId="user-target" />
        </I18nProvider>,
      );
    });
    await act(async () => {
      container
        .querySelector<HTMLButtonElement>('[data-testid="admin-thread-scraping-thread"]')
        ?.click();
    });

    // Only the first 50 messages mount; the expander reveals the rest.
    assert.match(container.textContent ?? "", /content-0/);
    assert.match(container.textContent ?? "", /content-49/);
    assert.doesNotMatch(container.textContent ?? "", /content-50/);
    const showMore = container.querySelector<HTMLButtonElement>(
      '[data-testid="admin-thread-scraping-show-more"]',
    );
    assert.ok(showMore, "a show-more control renders when the window is smaller than the artifact");
    await act(async () => showMore.click());
    assert.match(container.textContent ?? "", /content-59/);
    assert.equal(
      container.querySelector('[data-testid="admin-thread-scraping-show-more"]'),
      null,
      "the expander disappears once every message is visible",
    );
  } finally {
    act(() => root.unmount());
    container.remove();
    requests.fetchThreads.mockReset();
    requests.fetchArtifact.mockReset();
    requests.fetchRunArtifact.mockReset();
  }
});

test("download-thread button saves the artifact under a sanitized filename", async () => {
  requests.fetchThreads.mockResolvedValueOnce({
    threads: [{ thread_id: "thread/one", title: "One" }],
    next_cursor: null,
  });
  requests.fetchArtifact.mockResolvedValueOnce({
    schema: "ironclaw.thread_artifact.v1",
    generated_at: "2026-08-07T00:00:00Z",
    thread_id: "thread/one",
    messages: [
      {
        message_id: "message-one",
        sequence: 1,
        run_id: "run-one",
        kind: "assistant",
        status: "completed",
        content: "hello",
      },
    ],
    logs: {
      source: "operator",
      available: false,
      complete: false,
      truncated: false,
      entries: [],
    },
    redaction: { pipeline: "none", applied: false },
  });

  const container = document.createElement("div");
  document.body.append(container);
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const root = createTestRoot(container, queryClient);
  try {
    await act(async () => {
      root.render(
        <I18nProvider>
          <ThreadScrapingPanel userId="user-target" />
        </I18nProvider>,
      );
    });
    await act(async () => {
      container
        .querySelector<HTMLButtonElement>('[data-testid="admin-thread-scraping-thread"]')
        ?.click();
    });
    const downloadButton = container.querySelector<HTMLButtonElement>(
      '[data-testid="admin-thread-scraping-download-thread"]',
    );
    assert.ok(downloadButton, "the download button appears once an artifact is loaded");
    await act(async () => downloadButton.click());

    assert.equal(vi.mocked(saveBlob).mock.calls.length, 1);
    const [blob, filename] = vi.mocked(saveBlob).mock.calls[0] ?? [];
    // External input (the thread id) must not leak path separators or
    // whitespace into the download filename.
    assert.equal(filename, "ironclaw-thread-thread_one.json");
    const text = await blob.text();
    assert.match(text, /"thread_id":"thread\/one"/);
    assert.match(text, /"content":"hello"/);
  } finally {
    act(() => root.unmount());
    container.remove();
    requests.fetchThreads.mockReset();
    requests.fetchArtifact.mockReset();
    requests.fetchRunArtifact.mockReset();
    vi.mocked(saveBlob).mockReset();
  }
});

test("download-run button saves the artifact under a sanitized filename", async () => {
  requests.fetchThreads.mockResolvedValueOnce({
    threads: [{ thread_id: "thread-one", title: "One" }],
    next_cursor: null,
  });
  const artifactDeferred = deferred();
  requests.fetchArtifact.mockReturnValue(artifactDeferred.promise);
  requests.fetchRunArtifact.mockResolvedValueOnce({
    schema: "ironclaw.run_artifact.v1",
    generated_at: "2026-08-07T00:00:00Z",
    thread_id: "thread-one",
    run: { run_id: "run-one-123" },
    messages: [{ message_id: "message-one", sequence: 1, kind: "assistant", content: "run payload" }],
    logs: {
      source: "operator",
      available: false,
      complete: false,
      truncated: false,
      entries: [],
    },
    redaction: { pipeline: "none", applied: false },
  });

  const container = document.createElement("div");
  document.body.append(container);
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const root = createTestRoot(container, queryClient);
  try {
    await act(async () => {
      root.render(
        <I18nProvider>
          <ThreadScrapingPanel userId="user-target" />
        </I18nProvider>,
      );
    });
    await act(async () => {
      container
        .querySelector<HTMLButtonElement>('[data-testid="admin-thread-scraping-thread"]')
        ?.click();
    });
    await act(async () => {
      artifactDeferred.resolve({
        schema: "ironclaw.thread_artifact.v1",
        generated_at: "2026-08-07T00:00:00Z",
        thread_id: "thread-one",
        messages: [
          {
            message_id: "message-one",
            sequence: 1,
            run_id: "run-one-123",
            kind: "assistant",
            status: "completed",
            content: "hello",
          },
        ],
        logs: {
          source: "operator",
          available: false,
          complete: false,
          truncated: false,
          entries: [],
        },
        redaction: { pipeline: "none", applied: false },
      });
    });
    const runButton = Array.from(container.querySelectorAll<HTMLButtonElement>("button")).find(
      (button) => button.textContent?.includes("run-one-"),
    );
    assert.ok(runButton, "a run download button renders for the artifact's run");
    await act(async () => runButton.click());

    assert.equal(vi.mocked(saveBlob).mock.calls.length, 1);
    const [blob, filename] = vi.mocked(saveBlob).mock.calls[0] ?? [];
    assert.equal(filename, "ironclaw-run-run-one-123.json");
    const text = await blob.text();
    assert.match(text, /"run_id":"run-one-123"/);
  } finally {
    act(() => root.unmount());
    container.remove();
    requests.fetchThreads.mockReset();
    requests.fetchArtifact.mockReset();
    requests.fetchRunArtifact.mockReset();
    vi.mocked(saveBlob).mockReset();
  }
});
