// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { FlowList } from "./flow-list";

test("duplicate titles render and reconcile as distinct items", () => {
  const container = document.createElement("div");
  document.body.append(container);
  const consoleErrors: string[] = [];
  const originalConsoleError = console.error;
  console.error = (...args: unknown[]) => {
    consoleErrors.push(args.map(String).join(" "));
  };

  try {
    const root = createRoot(container);
    // State timelines repeat states (e.g. Running -> Paused twice), so the
    // list must not key rows on the title alone.
    act(() =>
      root.render(
        <FlowList
          items={[
            { title: "Running -> Paused", description: "first pause" },
            { title: "Paused -> Running", description: "resumed" },
            { title: "Running -> Paused", description: "second pause" },
          ]}
        />
      )
    );

    const rows = Array.from(container.firstElementChild?.children ?? []);
    assert.equal(rows.length, 3);
    assert.match(rows[0].textContent ?? "", /01.*Running -> Paused.*first pause/s);
    assert.match(rows[2].textContent ?? "", /03.*Running -> Paused.*second pause/s);
    assert.equal(
      consoleErrors.filter((message) => /same key|unique "key"/i.test(message)).length,
      0
    );
    act(() => root.unmount());
  } finally {
    console.error = originalConsoleError;
    container.remove();
  }
});

test("an explicit id wins over the positional key", () => {
  const container = document.createElement("div");
  document.body.append(container);
  try {
    const root = createRoot(container);
    act(() =>
      root.render(
        <FlowList
          items={[
            { id: "a", title: "Step" },
            { id: "b", title: "Step" },
          ]}
        />
      )
    );
    assert.equal(container.firstElementChild?.children.length, 2);
    act(() => root.unmount());
  } finally {
    container.remove();
  }
});
