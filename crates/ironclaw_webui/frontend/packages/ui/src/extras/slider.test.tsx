// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { renderIntoDocument } from "./test-helpers";
import { Slider } from "./slider";

test("Slider renders one thumb per value with slider semantics", () => {
  const rendered = renderIntoDocument(
    <Slider defaultValue={[40]} max={100} aria-label="Volume" />
  );
  try {
    const thumbs = rendered.container.querySelectorAll('[role="slider"]');
    assert.equal(thumbs.length, 1);
    assert.equal(thumbs[0].getAttribute("aria-valuenow"), "40");
    assert.equal(thumbs[0].getAttribute("aria-valuemax"), "100");
  } finally {
    rendered.unmount();
  }
});

test("Slider supports multi-thumb ranges", () => {
  const rendered = renderIntoDocument(
    <Slider defaultValue={[20, 70]} max={100} aria-label="Range" />
  );
  try {
    assert.equal(rendered.container.querySelectorAll('[role="slider"]').length, 2);
  } finally {
    rendered.unmount();
  }
});
