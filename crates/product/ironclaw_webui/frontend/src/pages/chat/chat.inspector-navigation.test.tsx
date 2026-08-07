// @vitest-environment jsdom

import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { MemoryRouter, useNavigate } from "react-router";
import { afterEach, beforeEach, test, vi } from "vitest";

import { Chat } from "./chat";

const eventStreams = vi.hoisted(() => [] as any[]);
const chatState = vi.hoisted(() => ({
  messages: [],
  isProcessing: false,
  pendingGate: null,
  pendingOnboarding: null,
  busyGateNotice: null,
  suggestions: [],
  sseStatus: "open",
  historyLoading: false,
  historyLoadError: null,
  hasMore: false,
  cooldownSeconds: 0,
  recoveryNotice: null,
  activeRun: { runId: "run-a", threadId: "thread-a", status: "running" },
  send: vi.fn(async () => ({})),
  runCommand: vi.fn(async () => ({})),
  cancelRun: vi.fn(async () => ({})),
  retryMessage: vi.fn(),
  approve: vi.fn(),
  recoverHistory: vi.fn(),
  loadMore: vi.fn(),
  setSuggestions: vi.fn(),
  submitAuthToken: vi.fn(async () => ({})),
  startOnboardingOAuth: vi.fn(),
  dismissOnboardingPairing: vi.fn(),
}));

vi.mock("event-source-plus", () => ({
  EventSourcePlus: class EventSourcePlus {
    hooks: Record<string, Function> = {};
    controller = { abort: vi.fn(), reconnect: vi.fn() };

    constructor() {
      eventStreams.push(this);
    }

    listen(hooks: Record<string, Function>) {
      this.hooks = hooks;
      hooks.onRequest?.({});
      return this.controller;
    }
  },
}));

vi.mock("./hooks/useChat", () => ({ useChat: () => chatState }));
vi.mock("./hooks/useChatCommands", () => ({ useChatCommands: () => [] }));
vi.mock("../../lib/i18n", () => ({ useT: () => (key: string) => key }));
vi.mock("../../lib/interface-preferences", () => ({
  useInterfacePreferences: () => ({ showChatLogsShortcut: false }),
}));
vi.mock("./lib/runtime-context", () => ({ buildRuntimeContext: () => ({}) }));
vi.mock("./components/empty-state", () => ({ EmptyState: () => null }));
vi.mock("./components/keyboard-shortcuts", () => ({ KeyboardShortcuts: () => null }));

let root: ReturnType<typeof createRoot> | null = null;

function NavigationHarness() {
  const navigate = useNavigate();
  return (
    <>
      <button data-testid="remove-debug" onClick={() => navigate("/chat")}>Remove debug</button>
      <Chat
        threads={[{ id: "thread-a" }]}
        activeThreadId="thread-a"
        onSelectThread={() => {}}
        isCreatingThread={false}
        gatewayStatus={{}}
        onConnectionStatusChange={() => {}}
      />
    </>
  );
}

beforeEach(() => {
  eventStreams.length = 0;
  sessionStorage.clear();
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => new Response(JSON.stringify({ snapshot: null }), {
      status: 200,
      headers: { "content-type": "application/json" },
    })),
  );
  Object.defineProperty(window, "innerWidth", {
    configurable: true,
    value: 1440,
  });
  root = createRoot(document.body.appendChild(document.createElement("div")));
});

afterEach(async () => {
  await act(async () => root?.unmount());
  document.body.replaceChildren();
  vi.unstubAllGlobals();
});

test("removing the debug query unmounts the inspector and releases its stream", async () => {
  await act(async () => {
    root?.render(
      <MemoryRouter initialEntries={["/chat?debug=true"]}>
        <NavigationHarness />
      </MemoryRouter>,
    );
    await import("./inspector/inspector-panel");
  });

  assert.ok(document.querySelector("[data-testid='inspector-panel']"));
  assert.equal(eventStreams.length, 1);
  const stream = eventStreams[0];

  await act(async () => {
    document.querySelector<HTMLButtonElement>("[data-testid='remove-debug']")?.click();
  });

  assert.equal(document.querySelector("[data-testid='inspector-panel']"), null);
  assert.equal(stream.controller.abort.mock.calls.at(-1)?.[0], "inspector disposed");
});
