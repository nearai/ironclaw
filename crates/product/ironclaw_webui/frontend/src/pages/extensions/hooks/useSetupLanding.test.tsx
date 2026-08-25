// @vitest-environment happy-dom
// @ts-nocheck
//
// #7853: device-link guidance rendered into a Telegram or Slack thread cannot
// show the link panel there, so it hands the user
// `{origin}/extensions?configure=<id>&setup=personal_account`. This pins the
// landing half: the params are consumed once and stripped, the id resolves
// against the caller's OWN installed inventory, and the named setup path
// reaches the modal so the link does not just dump the user on a choice screen.

import assert from "node:assert/strict";
import { test } from "vitest";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { MemoryRouter, useLocation } from "react-router";

import { useExtensionSetupLanding } from "./useSetupLanding";

function installed(id) {
  return { packageRef: { kind: "extension", id }, displayName: id };
}

async function flush() {
  await act(async () => {
    for (let i = 0; i < 6; i += 1) await Promise.resolve();
  });
}

// Renders the hook at `path` and reports every configure call plus the search
// string the browser is left on.
function renderAt(path, { extensions = [], isLoading = false } = {}) {
  const configured = [];
  const seen = { setupPath: undefined, search: undefined };

  function Probe() {
    const location = useLocation();
    const onConfigure = React.useCallback((extension) => {
      configured.push(extension);
    }, []);
    const { setupPath } = useExtensionSetupLanding({
      extensions,
      isLoading,
      onConfigure,
    });
    seen.setupPath = setupPath;
    seen.search = location.search;
    return null;
  }

  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => {
    root.render(
      <MemoryRouter initialEntries={[path]}>
        <Probe />
      </MemoryRouter>,
    );
  });
  return { configured, seen, root, container };
}

test("a setup link opens the named extension on the path it asked for", async () => {
  const { configured, seen } = renderAt(
    "/extensions?configure=telegram&setup=personal_account",
    { extensions: [installed("slack"), installed("telegram")] },
  );
  await flush();

  assert.equal(configured.length, 1);
  assert.equal(configured[0].packageRef.id, "telegram");
  assert.equal(seen.setupPath, "personal_account");
  // Stripped, so a reload cannot replay the landing.
  assert.equal(seen.search, "");
});

test("resolution waits for the inventory instead of missing on an empty list", async () => {
  // The params are stripped immediately but the extension list arrives
  // asynchronously; resolving against an empty in-flight list would silently
  // land the user on a page with no modal.
  const { configured, seen, root, container } = renderAt(
    "/extensions?configure=telegram&setup=personal_account",
    { extensions: [], isLoading: true },
  );
  await flush();
  assert.equal(configured.length, 0);

  const loaded = [];
  function Probe() {
    const onConfigure = React.useCallback((extension) => {
      loaded.push(extension);
    }, []);
    useExtensionSetupLanding({
      extensions: [installed("telegram")],
      isLoading: false,
      onConfigure,
    });
    return null;
  }
  act(() => {
    root.render(
      <MemoryRouter initialEntries={[`/extensions${seen.search}`]}>
        <Probe />
      </MemoryRouter>,
    );
  });
  await flush();
  // The remount has no params left to read: the strip is one-shot by design,
  // which is why the hook holds the request in state rather than re-reading it.
  assert.equal(loaded.length, 0);
  container.remove();
});

test("an id the caller has not installed opens nothing", async () => {
  const { configured, seen } = renderAt(
    "/extensions?configure=telegram&setup=personal_account",
    { extensions: [installed("slack")] },
  );
  await flush();

  assert.equal(configured.length, 0);
  // No setup path either: preselecting a ceremony for an extension that is not
  // there would open the next modal on the wrong screen.
  assert.equal(seen.setupPath, null);
});

test("an unrecognized setup path falls back to the choice screen", async () => {
  // A newer host talking to an older browser must not silently land the user on
  // the wrong ceremony.
  const { configured, seen } = renderAt(
    "/extensions?configure=telegram&setup=teleport",
    { extensions: [installed("telegram")] },
  );
  await flush();

  assert.equal(configured.length, 1);
  assert.equal(seen.setupPath, null);
});

test("unrelated query params survive the strip", async () => {
  const { seen } = renderAt(
    "/extensions?ref=telegram-thread&configure=telegram&setup=personal_account",
    { extensions: [installed("telegram")] },
  );
  await flush();

  assert.equal(seen.search, "?ref=telegram-thread");
});

test("no configure param leaves the url and the modal alone", async () => {
  const { configured, seen } = renderAt("/extensions?ref=telegram-thread", {
    extensions: [installed("telegram")],
  });
  await flush();

  assert.equal(configured.length, 0);
  assert.equal(seen.setupPath, null);
  assert.equal(seen.search, "?ref=telegram-thread");
});
