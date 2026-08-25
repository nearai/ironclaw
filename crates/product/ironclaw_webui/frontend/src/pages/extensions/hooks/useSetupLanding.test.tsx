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
    // Mirrors ExtensionsPage: the extension `onConfigure` resolves becomes
    // the modal's `selected` extension on the next render.
    const [selected, setSelected] = React.useState(null);
    const onConfigure = React.useCallback((extension) => {
      configured.push(extension);
      setSelected(extension);
    }, []);
    const { setupPath } = useExtensionSetupLanding({
      extensions,
      isLoading,
      onConfigure,
      selected,
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

// The shape the server actually sends (`package_ref`), as opposed to the
// post-Configure shape. Every test below used the camelCase form, which is why
// six green tests still let the deep link open nothing against a live
// deployment: the fixture had been written to match the hook, not the API.
function rawApiItem(id) {
  return { package_ref: { kind: "extension", id }, display_name: id };
}

test("a raw API list item resolves — the shape the server actually sends", async () => {
  const { configured, seen } = renderAt(
    "/extensions?configure=telegram&setup=personal_account",
    { extensions: [rawApiItem("slack"), rawApiItem("telegram")] },
  );
  await flush();

  assert.equal(configured.length, 1, "a snake_case package_ref must still resolve");
  assert.equal(configured[0].package_ref.id, "telegram");
  assert.equal(seen.setupPath, "personal_account");
});

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
  // land the user on a page with no modal. This drives the SAME hook
  // instance from `isLoading: true` on an empty list to `isLoading: false`
  // on a populated one — no remount — so it actually exercises the held
  // request, not just the one-shot URL strip.
  const configured = [];
  const onConfigure = (extension) => configured.push(extension);
  const path = "/extensions?configure=telegram&setup=personal_account";

  function Probe({ extensions, isLoading }) {
    useExtensionSetupLanding({ extensions, isLoading, onConfigure });
    return null;
  }

  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);

  act(() => {
    root.render(
      <MemoryRouter initialEntries={[path]}>
        <Probe extensions={[]} isLoading={true} />
      </MemoryRouter>,
    );
  });
  await flush();
  assert.equal(configured.length, 0);

  // Same tree, same component instance: only the inventory props change.
  act(() => {
    root.render(
      <MemoryRouter initialEntries={[path]}>
        <Probe extensions={[installed("telegram")]} isLoading={false} />
      </MemoryRouter>,
    );
  });
  await flush();

  assert.equal(configured.length, 1);
  assert.equal(configured[0].packageRef.id, "telegram");
  container.remove();
});

test("a reload after the strip has nothing left to replay", async () => {
  // The strip is one-shot by design, which is why the hook holds the
  // request in state rather than re-reading the URL: a genuine reload gets a
  // fresh hook instance and a search string with nothing left to consume.
  const { seen, container: firstContainer } = renderAt(
    "/extensions?configure=telegram&setup=personal_account",
    { extensions: [], isLoading: true },
  );
  await flush();

  const { configured, container } = renderAt(`/extensions${seen.search}`, {
    extensions: [installed("telegram")],
    isLoading: false,
  });
  await flush();
  assert.equal(configured.length, 0);
  firstContainer.remove();
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

test("a later Configure action does not inherit the consumed setup path", async () => {
  // Mirrors ExtensionsPage: `selected` is whatever the caller is about to
  // render the modal for, and `clearSetupPath` runs when that modal closes.
  const configured = [];
  const onConfigure = (extension) => configured.push(extension);
  const path = "/extensions?configure=telegram&setup=personal_account";
  const extensions = [installed("telegram"), installed("other")];
  let api = null;

  function Probe({ selected }) {
    api = useExtensionSetupLanding({
      extensions,
      isLoading: false,
      onConfigure,
      selected,
    });
    return null;
  }

  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);

  act(() => {
    root.render(
      <MemoryRouter initialEntries={[path]}>
        <Probe selected={null} />
      </MemoryRouter>,
    );
  });
  await flush();
  assert.equal(configured.length, 1);
  assert.equal(configured[0].packageRef.id, "telegram");

  // The deep link's own modal — selected is the extension it resolved to —
  // gets the path.
  act(() => {
    root.render(
      <MemoryRouter initialEntries={[path]}>
        <Probe selected={installed("telegram")} />
      </MemoryRouter>,
    );
  });
  await flush();
  assert.equal(api.setupPath, "personal_account");

  // A Configure click on a DIFFERENT extension while the path is still held
  // must not inherit it, even before the deep-link modal is explicitly
  // closed.
  act(() => {
    root.render(
      <MemoryRouter initialEntries={[path]}>
        <Probe selected={installed("other")} />
      </MemoryRouter>,
    );
  });
  await flush();
  assert.equal(api.setupPath, null);

  // Closing the deep-link modal clears the stored path, so reopening the
  // SAME extension later lands on the choice screen too.
  act(() => {
    api.clearSetupPath();
  });
  act(() => {
    root.render(
      <MemoryRouter initialEntries={[path]}>
        <Probe selected={installed("telegram")} />
      </MemoryRouter>,
    );
  });
  await flush();
  assert.equal(api.setupPath, null);

  container.remove();
});
