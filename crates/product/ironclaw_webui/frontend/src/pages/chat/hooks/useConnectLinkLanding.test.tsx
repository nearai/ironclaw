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
  fetchOauthFlowStatus: vi.fn(),
  installExtension: vi.fn(),
  startExtensionOauth: vi.fn(),
}));

vi.mock("../../extensions/lib/extensions-api", () => api);

import { useConnectLinkLanding } from "./useConnectLinkLanding";

function fakePopup() {
  return { closed: false, opener: null, location: { href: "about:blank" } };
}

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
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
