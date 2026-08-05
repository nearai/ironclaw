// @vitest-environment jsdom

import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, test, vi } from "vitest";

import { INSPECTOR_HEALTH } from "./inspector-state";
import { InspectorPanel } from "./inspector-panel";

const inspectorCalls = vi.hoisted(() => [] as any[]);

vi.mock("./useInspector", () => ({
  useInspector: (input: unknown) => {
    inspectorCalls.push(input);
    return {
      snapshot: null,
      updates: [],
      health: INSPECTOR_HEALTH.CONNECTED,
      error: null,
      lastCursor: null,
    };
  },
}));

let root: ReturnType<typeof createRoot> | null = null;

function setViewport(width: number) {
  Object.defineProperty(window, "innerWidth", {
    configurable: true,
    value: width,
  });
  window.dispatchEvent(new Event("resize"));
}

beforeEach(() => {
  inspectorCalls.length = 0;
  sessionStorage.clear();
  setViewport(1440);
  root = createRoot(document.body.appendChild(document.createElement("div")));
});

afterEach(async () => {
  await act(async () => root?.unmount());
  document.body.replaceChildren();
});

test("panel switches tabs, closes, reopens, overlays tablets, and hides on mobile", async () => {
  await act(async () =>
    root?.render(<InspectorPanel threadId="thread-a" runId="run-a" />),
  );
  const panel = document.querySelector<HTMLElement>("[data-testid='inspector-panel']");
  assert.equal(panel?.dataset.layout, "sidebar");
  assert.equal(document.querySelector("[data-testid='inspector-health']")?.textContent, "Live");

  await act(async () =>
    document.querySelector<HTMLButtonElement>("[data-testid='inspector-tab-stats']")?.click(),
  );
  assert.equal(
    document.querySelector("[data-testid='inspector-tab-stats']")?.getAttribute("aria-selected"),
    "true",
  );

  await act(async () =>
    document.querySelector<HTMLButtonElement>("[data-testid='inspector-close']")?.click(),
  );
  assert.equal(document.querySelector("[data-testid='inspector-panel']"), null);
  assert.ok(document.querySelector("[data-testid='inspector-open']"));

  await act(async () =>
    document.querySelector<HTMLButtonElement>("[data-testid='inspector-open']")?.click(),
  );
  assert.equal(
    document.querySelector("[data-testid='inspector-tab-stats']")?.getAttribute("aria-selected"),
    "true",
  );

  await act(async () => setViewport(900));
  assert.equal(
    document.querySelector<HTMLElement>("[data-testid='inspector-panel']")?.dataset.layout,
    "overlay",
  );

  await act(async () => setViewport(500));
  assert.equal(document.querySelector("[data-testid='inspector-panel']"), null);
  assert.equal(inspectorCalls.at(-1)?.enabled, false);
});
