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
  assert.equal(api.fetchExtensionSetup.mock.calls[0][0].id, "slack");
  assert.equal(api.startExtensionOauth.mock.calls[0][0].id, "slack");
  assert.equal(api.startExtensionOauth.mock.calls[0][1].name, "slack_oauth");
  assert.equal(popup.location.href, "https://slack.com/oauth/authorize?client_id=abc");
});

test("dismissing clears the landing state", async () => {
  renderAt("/chat?connect=slack");
  await flush();
  assert.ok(latest.connectLanding);

  act(() => latest.dismissConnectLanding());
  await flush();

  assert.equal(latest.connectLanding, null);
});
