// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { click, renderIntoDocument } from "./test-helpers";
import { Toggle } from "./toggle";

test("Toggle flips aria-pressed on click", () => {
  const changes: boolean[] = [];
  const rendered = renderIntoDocument(
    <Toggle aria-label="Pin" onPressedChange={(next) => changes.push(next)}>Pin</Toggle>
  );
  try {
    const button = rendered.container.querySelector("button");
    assert.ok(button);
    assert.equal(button.getAttribute("aria-pressed"), "false");
    click(button);
    assert.equal(button.getAttribute("aria-pressed"), "true");
    assert.deepEqual(changes, [true]);
  } finally {
    rendered.unmount();
  }
});
