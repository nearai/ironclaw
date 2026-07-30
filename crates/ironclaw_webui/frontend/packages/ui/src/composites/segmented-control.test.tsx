// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { SegmentedControl } from "./segmented-control";

const OPTIONS = [
  { value: "all", label: "All" },
  { value: "active", label: "Active" },
  { value: "failed", label: "Failed", disabled: true },
];

test("SegmentedControl marks the selected option and reports changes", () => {
  const container = document.createElement("div");
  document.body.append(container);
  const changes: string[] = [];

  try {
    const root = createRoot(container);
    act(() =>
      root.render(
        <SegmentedControl
          label="Filter"
          options={OPTIONS}
          value="all"
          onChange={(value) => changes.push(value)}
          optionTestId="filter-option"
        />
      )
    );

    const group = container.querySelector('[role="group"]');
    assert.ok(group);
    assert.equal(group.getAttribute("aria-label"), "Filter");

    const buttons = Array.from(container.querySelectorAll("button"));
    assert.equal(buttons.length, 3);
    assert.equal(buttons[0].getAttribute("aria-pressed"), "true");
    assert.equal(buttons[1].getAttribute("aria-pressed"), "false");
    assert.equal(buttons[0].dataset.value, "all");
    assert.equal(buttons[0].dataset.testid, "filter-option");
    assert.ok((buttons[2] as HTMLButtonElement).disabled);

    act(() => buttons[1].click());
    assert.deepEqual(changes, ["active"]);
  } finally {
    container.remove();
  }
});
