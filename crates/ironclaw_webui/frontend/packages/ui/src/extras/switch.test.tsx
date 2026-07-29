// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { click, renderIntoDocument } from "./test-helpers";
import { Switch } from "./switch";

test("Switch toggles aria-checked on click", () => {
  const changes: boolean[] = [];
  const rendered = renderIntoDocument(
    <Switch aria-label="Notifications" onCheckedChange={(next) => changes.push(next)} />
  );
  try {
    const control = rendered.container.querySelector('[role="switch"]');
    assert.ok(control);
    assert.equal(control.getAttribute("aria-checked"), "false");
    click(control);
    assert.equal(control.getAttribute("aria-checked"), "true");
    assert.deepEqual(changes, [true]);
  } finally {
    rendered.unmount();
  }
});

test("disabled Switch does not toggle", () => {
  const changes: boolean[] = [];
  const rendered = renderIntoDocument(
    <Switch aria-label="Locked" disabled onCheckedChange={(next) => changes.push(next)} />
  );
  try {
    const control = rendered.container.querySelector('[role="switch"]');
    assert.ok(control);
    click(control);
    assert.equal(control.getAttribute("aria-checked"), "false");
    assert.deepEqual(changes, []);
  } finally {
    rendered.unmount();
  }
});
