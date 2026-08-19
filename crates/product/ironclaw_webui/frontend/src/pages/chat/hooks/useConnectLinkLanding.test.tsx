// @vitest-environment happy-dom
// @ts-nocheck
//
// #7681: `?connect=<extension>` lands an OAuth-strategy channel's smart
// connect link on `/chat`. This pins the landing half: the param is detected
// and stripped on first render (so a reload doesn't replay it), and clicking
// through drives the SAME setup -> oauth-start -> popup sequence the
// Settings/Extensions "Connect" button uses.

import assert from "node:assert/strict";
import { afterEach, beforeEach, test, vi } from "vitest";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { MemoryRouter, useLocation } from "react-router";
import "../../../i18n/en";
import { I18nProvider } from "../../../lib/i18n";

const api = vi.hoisted(() => ({
  fetchExtensionSetup: vi.fn(),
  fetchExtensionRegistry: vi.fn(),
  fetchExtensions: vi.fn(),
  fetchOauthFlowStatus: vi.fn(),
  installExtension: vi.fn(),
  startExtensionOauth: vi.fn(),
}));

// The server-published inventory the `connect` param is resolved against: only
// an extension the server lists with an OAuth channel connection may be
// installed and connected from a link.
function oauthChannelEntry(id, displayName) {
  return {
    package_ref: { kind: "extension", id },
    display_name: displayName,
    surfaces: [{ kind: "channel", connection: { channel: id, strategy: "oauth" } }],
  };
}

vi.mock("../../extensions/lib/extensions-api", () => api);

// The hook loads `lib/connect-link-flow` through a dynamic `import()` so the
// machinery stays out of the eager /chat bundle. Importing it statically here
// puts it in the module registry, so that `import()` resolves on a microtask
// and the flush helper below stays deterministic.
import "../lib/connect-link-flow";
import { useConnectLinkLanding } from "./useConnectLinkLanding";

function fakePopup() {
  return { closed: false, opener: null, location: { href: "about:blank" } };
}

async function flush() {
  await act(async () => {
    for (let i = 0; i < 6; i += 1) await Promise.resolve();
  });
}

let container;
let root;
let latest;
let originalOpen;

beforeEach(() => {
  vi.clearAllMocks();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  latest = null;
  originalOpen = window.open;
  api.fetchExtensions.mockResolvedValue({ extensions: [] });
  api.fetchExtensionRegistry.mockResolvedValue({
    entries: [oauthChannelEntry("slack", "Slack")],
  });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  window.open = originalOpen;
});

function Probe() {
  const hook = useConnectLinkLanding();
  const location = useLocation();
  latest = { ...hook, search: location.search };
  return null;
}

function renderAt(path) {
  act(() => {
    root.render(
      <MemoryRouter initialEntries={[path]}>
        <I18nProvider>
          <Probe />
        </I18nProvider>
      </MemoryRouter>,
    );
  });
}

test("detects and strips the connect query param on landing", async () => {
  renderAt("/chat?connect=slack");
  await flush();

  assert.equal(latest.connectLanding?.extensionName, "slack");
  assert.equal(latest.connectLanding?.strategy, "oauth");
  assert.equal(
    latest.connectLanding?.submitLabel,
    "Continue to connect Slack",
    "the card asks for an explicit confirming click, not an auto-redirect",
  );
  assert.equal(latest.search, "", "the connect param is stripped so a reload cannot replay it");
});

test("preserves sibling query params while stripping connect", async () => {
  renderAt("/chat?foo=bar&connect=slack");
  await flush();

  assert.equal(latest.connectLanding?.extensionName, "slack");
  assert.equal(latest.search, "?foo=bar");
});

test("no connect param means no landing state", async () => {
  renderAt("/chat");
  await flush();

  assert.equal(latest.connectLanding, null);
});

test("starting the connect flow drives setup, oauth start, and popup in order", async () => {
  const popup = fakePopup();
  window.open = vi.fn(() => popup);
  api.installExtension.mockResolvedValue({ success: true });
  api.fetchExtensionSetup.mockResolvedValue({
    secrets: [{ name: "slack_oauth", setup: { kind: "oauth" } }],
  });
  api.startExtensionOauth.mockResolvedValue({
    success: true,
    authorization_url: "https://slack.com/oauth/authorize?client_id=abc",
    flow_id: "flow-1",
  });

  renderAt("/chat?connect=slack");
  await flush();

  await act(async () => {
    await latest.startConnectLinkOAuth();
  });

  assert.equal(window.open.mock.calls[0][0], "about:blank");
  // Install must precede setup: the backend fails `oauth/start` closed for an
  // extension absent from the caller's inventory, which is the normal state
  // for someone arriving from a channel nudge (#7681 manual verification).
  assert.equal(api.installExtension.mock.calls[0][0].id, "slack");
  assert.ok(
    api.installExtension.mock.invocationCallOrder[0] <
      api.fetchExtensionSetup.mock.invocationCallOrder[0],
    "installExtension must be called before fetchExtensionSetup",
  );
  assert.equal(api.fetchExtensionSetup.mock.calls[0][0].id, "slack");
  assert.equal(api.startExtensionOauth.mock.calls[0][0].id, "slack");
  assert.equal(api.startExtensionOauth.mock.calls[0][1].name, "slack_oauth");
  assert.equal(popup.location.href, "https://slack.com/oauth/authorize?client_id=abc");
});

// The OAuth callback commonly lands on a different origin than the opener (a
// tunnelled callback against a 127.0.0.1 app), where the same-origin
// broadcast never arrives. Without the durable status poll the card spins
// forever — observed live before this backstop existed.
async function startFlow() {
  const popup = fakePopup();
  window.open = vi.fn(() => popup);
  api.installExtension.mockResolvedValue({ success: true });
  api.fetchExtensionSetup.mockResolvedValue({
    secrets: [{ name: "slack_oauth", setup: { kind: "oauth" } }],
  });
  api.startExtensionOauth.mockResolvedValue({
    success: true,
    authorization_url: "https://slack.com/oauth/authorize?client_id=abc",
    flow_id: "flow-1",
    callback_scope: { invocation_id: "inv-1" },
  });
  renderAt("/chat?connect=slack");
  await flush();
  await act(async () => {
    await latest.startConnectLinkOAuth();
  });
}

test("polled completion closes the card when no same-origin broadcast arrives", async () => {
  vi.useFakeTimers();
  try {
    await startFlow();
    assert.ok(latest.connectLanding, "card is still open while connecting");

    api.fetchOauthFlowStatus.mockResolvedValue({ status: "completed" });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2100);
    });

    assert.deepEqual(api.fetchOauthFlowStatus.mock.calls[0], ["flow-1", "inv-1"]);
    assert.equal(latest.connectLanding, null, "card closes on polled completion");
  } finally {
    vi.useRealTimers();
  }
});

test("a terminal failure status stops the spinner with a retryable error", async () => {
  vi.useFakeTimers();
  try {
    await startFlow();
    api.fetchOauthFlowStatus.mockResolvedValue({ status: "failed" });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2100);
    });

    assert.ok(latest.connectLanding, "the card stays so the user can retry");
    assert.ok(
      latest.connectLanding.oauthError,
      "oauthError is what makes the card exit its spinner and show the error",
    );
  } finally {
    vi.useRealTimers();
  }
});

test("dismissing clears the landing state", async () => {
  renderAt("/chat?connect=slack");
  await flush();
  assert.ok(latest.connectLanding);

  act(() => latest.dismissConnectLanding());
  await flush();

  assert.equal(latest.connectLanding, null);
});

// The `connect` param arrives on a link anyone can craft, so it must not be
// able to install an attacker-chosen extension and start its OAuth flow. Only
// an extension the server itself lists with an OAuth channel connection is
// accepted; everything else is ignored silently.
test("an unknown connect param renders no card and installs nothing", async () => {
  renderAt("/chat?connect=google-drive");
  await flush();

  assert.equal(latest.connectLanding, null);
  assert.equal(api.installExtension.mock.calls.length, 0);
});

test("a non-OAuth channel connect param renders no card and installs nothing", async () => {
  api.fetchExtensionRegistry.mockResolvedValue({
    entries: [
      {
        package_ref: { kind: "extension", id: "telegram" },
        display_name: "Telegram",
        surfaces: [
          { kind: "channel", connection: { channel: "telegram", strategy: "inbound_proof_code" } },
        ],
      },
    ],
  });

  renderAt("/chat?connect=telegram");
  await flush();

  assert.equal(latest.connectLanding, null);
  assert.equal(api.installExtension.mock.calls.length, 0);
});

test("the button label comes from the server display name, not the raw param", async () => {
  api.fetchExtensionRegistry.mockResolvedValue({
    entries: [oauthChannelEntry("slack", "Slack Workspace")],
  });

  renderAt("/chat?connect=slack");
  await flush();

  assert.equal(latest.connectLanding?.submitLabel, "Continue to connect Slack Workspace");
});

// Retrying after a failure must produce a visible prop change even when the
// second failure is identical: the card only leaves its spinner when the
// `oauthError` VALUE changes.
test("two consecutive identical failures both surface the error state", async () => {
  vi.useFakeTimers();
  try {
    await startFlow();
    api.fetchOauthFlowStatus.mockResolvedValue({ status: "failed" });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2100);
    });
    const firstError = latest.connectLanding?.oauthError;
    assert.ok(firstError, "the first failure surfaces an error");

    await act(async () => {
      await latest.startConnectLinkOAuth();
    });
    assert.equal(
      latest.connectLanding?.oauthError,
      null,
      "the retry clears the stale error so an identical second failure is a real change",
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2100);
    });
    assert.equal(latest.connectLanding?.oauthError, firstError, "the second failure re-surfaces");
  } finally {
    vi.useRealTimers();
  }
});

test("an abandoned flow times out and stops polling", async () => {
  vi.useFakeTimers();
  try {
    await startFlow();
    api.fetchOauthFlowStatus.mockResolvedValue({ status: "pending" });

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10 * 60 * 1000 + 5000);
    });

    assert.equal(
      latest.connectLanding?.oauthError,
      "Authorization timed out. Try connecting again.",
    );
    const callsAtTimeout = api.fetchOauthFlowStatus.mock.calls.length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });
    assert.equal(
      api.fetchOauthFlowStatus.mock.calls.length,
      callsAtTimeout,
      "the watcher stops polling once the flow is abandoned",
    );
  } finally {
    vi.useRealTimers();
  }
});

test("a blocked popup fails the connect click before any install", async () => {
  window.open = vi.fn(() => null);
  renderAt("/chat?connect=slack");
  await flush();

  await act(async () => {
    await assert.rejects(
      () => latest.startConnectLinkOAuth(),
      /Authorization popup was blocked/,
    );
  });
  assert.equal(api.installExtension.mock.calls.length, 0);
});

// A rejected install answers with `{ success: false }` rather than throwing, so
// the flow has to read the backend's verdict: continuing would start OAuth for
// an extension that was never installed (`tool-evidence.md` — UI success must
// follow backend evidence).
test("an install the backend rejects stops before setup and oauth start", async () => {
  const popup = fakePopup();
  window.open = vi.fn(() => popup);
  popup.close = () => {
    popup.closed = true;
  };
  api.installExtension.mockResolvedValue({ success: false, message: "install refused" });

  renderAt("/chat?connect=slack");
  await flush();

  await act(async () => {
    await assert.rejects(() => latest.startConnectLinkOAuth(), /install refused/);
  });
  assert.equal(api.fetchExtensionSetup.mock.calls.length, 0);
  assert.equal(api.startExtensionOauth.mock.calls.length, 0);
  assert.equal(popup.closed, true, "no about:blank window is left open");
});

test("a failed install closes the placeholder popup instead of leaking it", async () => {
  const popup = fakePopup();
  window.open = vi.fn(() => popup);
  popup.close = () => {
    popup.closed = true;
  };
  api.installExtension.mockRejectedValue(new Error("install refused"));

  renderAt("/chat?connect=slack");
  await flush();

  await act(async () => {
    await assert.rejects(() => latest.startConnectLinkOAuth(), /install refused/);
  });
  assert.equal(popup.closed, true, "no about:blank window is left open");
  assert.equal(api.startExtensionOauth.mock.calls.length, 0);
});
