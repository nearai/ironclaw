// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { click, renderIntoDocument } from "./test-helpers";
import { ToggleGroup, ToggleGroupItem } from "./toggle-group";

test("single-select ToggleGroup uses radio semantics", () => {
  const values: string[] = [];
  const rendered = renderIntoDocument(
    <ToggleGroup type="single" defaultValue="list" onValueChange={(value) => values.push(value)} aria-label="View">
      <ToggleGroupItem value="list">List</ToggleGroupItem>
      <ToggleGroupItem value="grid">Grid</ToggleGroupItem>
    </ToggleGroup>
  );
  try {
    const items = rendered.container.querySelectorAll('[role="radio"]');
    assert.equal(items.length, 2);
    assert.equal(items[0].getAttribute("aria-checked"), "true");
    click(items[1]);
    assert.equal(items[1].getAttribute("aria-checked"), "true");
    assert.deepEqual(values, ["grid"]);
  } finally {
    rendered.unmount();
  }
});

test("multi-select ToggleGroup uses pressed-button semantics", () => {
  const rendered = renderIntoDocument(
    <ToggleGroup type="multiple" defaultValue={["bold"]} aria-label="Formatting">
      <ToggleGroupItem value="bold">B</ToggleGroupItem>
      <ToggleGroupItem value="italic">I</ToggleGroupItem>
    </ToggleGroup>
  );
  try {
    const buttons = rendered.container.querySelectorAll("button");
    assert.equal(buttons[0].getAttribute("aria-pressed"), "true");
    assert.equal(buttons[1].getAttribute("aria-pressed"), "false");
    click(buttons[1]);
    assert.equal(buttons[1].getAttribute("aria-pressed"), "true");
    assert.equal(buttons[0].getAttribute("aria-pressed"), "true", "multiple keeps both on");
  } finally {
    rendered.unmount();
  }
});
