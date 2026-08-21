// @vitest-environment happy-dom
import assert from "node:assert/strict";
import type { Decorator } from "@storybook/react-vite";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { test, vi } from "vitest";

import { withStubbedFetch } from "./storybook-decorators";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

/**
 * Storybook renders every decorator as its own component, which is what makes a
 * story swap an unmount + mount rather than a re-render of one instance. Give
 * each decorator a distinct wrapper type so the test reproduces that handoff:
 * React renders the incoming wrapper BEFORE running the outgoing wrapper's
 * effect cleanup, the ordering that used to let the new story run on the old
 * story's routes and then lose its stub to the old cleanup.
 */
function storyComponent(decorator: Decorator, story: () => React.ReactNode) {
  return function DecoratedStory() {
    return decorator(story as never, {} as never) as React.ReactNode;
  };
}

/** Fetches `url` once on mount and records what came back. */
function Probe({ url, seen }: { url: string; seen: string[] }) {
  React.useEffect(() => {
    void (async () => {
      const response = await window.fetch(url);
      seen.push(await response.text());
    })();
  }, [url, seen]);
  return <span>{url}</span>;
}

test("a story swap hands the fetch stub over without exposing the real fetch", async () => {
  const realFetch = vi.fn(async () => new Response("REAL BACKEND"));
  const originalFetch = window.fetch;
  window.fetch = realFetch as unknown as typeof window.fetch;

  const seen: string[] = [];
  const Alpha = storyComponent(
    withStubbedFetch([{ match: "/alpha", json: { from: "alpha" } }]),
    () => <Probe url="https://host/alpha" seen={seen} />,
  );
  const Beta = storyComponent(
    withStubbedFetch([{ match: "/beta", json: { from: "beta" } }]),
    () => <Probe url="https://host/beta" seen={seen} />,
  );

  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);

  try {
    await act(async () => root.render(<Alpha />));
    assert.deepEqual(seen, ['{"from":"alpha"}']);

    // The swap: Beta renders while Alpha is still mounted, then Alpha's
    // cleanup runs. Beta must serve its OWN routes, and Alpha's cleanup must
    // not strip Beta's stub back to the real fetch.
    await act(async () => root.render(<Beta />));
    assert.deepEqual(seen, ['{"from":"alpha"}', '{"from":"beta"}']);
    assert.equal(realFetch.mock.calls.length, 0, "no story request reached the real fetch");

    // A route Beta does not declare is a loud failure, not a quiet trip to the
    // network: an unmatched request must never become live backend access.
    await assert.rejects(
      () => window.fetch("https://host/unmatched"),
      /unmatched GET https:\/\/host\/unmatched/,
    );
    assert.equal(realFetch.mock.calls.length, 0);

    // Unmounting the last owner puts the real fetch back: a request the stub
    // WOULD have matched now reaches the backend. (`window.fetch` is restored
    // to a bound copy of the real one, so this asserts reachability rather
    // than object identity.)
    await act(async () => root.unmount());
    await window.fetch("https://host/beta");
    assert.equal(realFetch.mock.calls.length, 1, "the last stub restores the real fetch");
  } finally {
    window.fetch = originalFetch;
    container.remove();
  }
});

test("passthrough is opt-in, and only reaches the network for unmatched routes", async () => {
  const realFetch = vi.fn(async () => new Response("REAL BACKEND"));
  const originalFetch = window.fetch;
  window.fetch = realFetch as unknown as typeof window.fetch;

  const seen: string[] = [];
  const Open = storyComponent(
    withStubbedFetch([{ match: "/stubbed", json: { from: "stub" } }], { passthrough: true }),
    () => <Probe url="https://host/stubbed" seen={seen} />,
  );

  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);

  try {
    await act(async () => root.render(<Open />));
    // A declared route still wins over the network...
    assert.deepEqual(seen, ['{"from":"stub"}']);
    assert.equal(realFetch.mock.calls.length, 0);

    // ...and only what the routes do not cover falls through.
    await act(async () => {
      await window.fetch("https://host/elsewhere");
    });
    assert.equal(realFetch.mock.calls.length, 1);
  } finally {
    await act(async () => root.unmount());
    window.fetch = originalFetch;
    container.remove();
  }
});

test("the render phase never installs the stub — only a committed mount does", () => {
  const realFetch = vi.fn(async () => new Response("REAL BACKEND"));
  const originalFetch = window.fetch;
  window.fetch = realFetch as unknown as typeof window.fetch;

  const Alpha = storyComponent(
    withStubbedFetch([{ match: "/alpha", json: { from: "alpha" } }]),
    () => <span>alpha</span>,
  );

  // `renderToStaticMarkup` runs the render phase and stops, which is what an
  // abandoned render (interrupted, suspended, or thrown out) does. Such a
  // render schedules no cleanup, so a render-phase install would strand a stub
  // on `window.fetch` with nothing left to ever remove it.
  const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
  try {
    renderToStaticMarkup(<Alpha />);
    assert.equal(window.fetch, realFetch, "an uncommitted render must not touch window.fetch");
  } finally {
    consoleError.mockRestore();
    window.fetch = originalFetch;
  }
});

test("a Request input is matched on its own method, not treated as GET", async () => {
  const realFetch = vi.fn(async () => new Response("REAL BACKEND"));
  const originalFetch = window.fetch;
  window.fetch = realFetch as unknown as typeof window.fetch;

  const Minting = storyComponent(
    withStubbedFetch([
      { match: "/pairing", json: { from: "status" } },
      { match: "/pairing", method: "POST", json: { from: "mint" } },
    ]),
    () => <span>minting</span>,
  );

  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);

  try {
    await act(async () => root.render(<Minting />));

    // The method rides on the Request, not on `init` — reading only `init`
    // would fall through to the GET route and serve the wrong body.
    const posted = await window.fetch(new Request("https://host/pairing", { method: "POST" }));
    assert.deepEqual(await posted.json(), { from: "mint" });

    const got = await window.fetch(new Request("https://host/pairing"));
    assert.deepEqual(await got.json(), { from: "status" });
    assert.equal(realFetch.mock.calls.length, 0);
  } finally {
    await act(async () => root.unmount());
    window.fetch = originalFetch;
    container.remove();
  }
});
