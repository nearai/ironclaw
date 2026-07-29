// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { click, renderIntoDocument } from "./test-helpers";
import { Checkbox } from "./checkbox";

test("Checkbox toggles checked state on click", () => {
  const changes: (boolean | "indeterminate")[] = [];
  const rendered = renderIntoDocument(
    <Checkbox aria-label="Enable" onCheckedChange={(next) => changes.push(next)} />
  );
  try {
    const checkbox = rendered.container.querySelector('[role="checkbox"]');
    assert.ok(checkbox, "renders role=checkbox");
    assert.equal(checkbox.getAttribute("aria-checked"), "false");
    click(checkbox);
    assert.equal(checkbox.getAttribute("aria-checked"), "true");
    assert.deepEqual(changes, [true]);
  } finally {
    rendered.unmount();
  }
});

test("Checkbox renders the indeterminate dash state", () => {
  const rendered = renderIntoDocument(
    <Checkbox aria-label="Partial" checked="indeterminate" />
  );
  try {
    const checkbox = rendered.container.querySelector('[role="checkbox"]');
    assert.equal(checkbox?.getAttribute("aria-checked"), "mixed");
  } finally {
    rendered.unmount();
  }
});
