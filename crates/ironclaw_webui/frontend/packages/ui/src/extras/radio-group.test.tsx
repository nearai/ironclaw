// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { click, renderIntoDocument } from "./test-helpers";
import { RadioGroup, RadioGroupItem } from "./radio-group";

test("RadioGroup selects exactly one item on click", () => {
  const values: string[] = [];
  const rendered = renderIntoDocument(
    <RadioGroup defaultValue="a" onValueChange={(value) => values.push(value)} aria-label="Mode">
      <RadioGroupItem value="a" aria-label="A" />
      <RadioGroupItem value="b" aria-label="B" />
    </RadioGroup>
  );
  try {
    assert.ok(rendered.container.querySelector('[role="radiogroup"]'));
    const radios = rendered.container.querySelectorAll('[role="radio"]');
    assert.equal(radios.length, 2);
    assert.equal(radios[0].getAttribute("aria-checked"), "true");
    click(radios[1]);
    assert.equal(radios[0].getAttribute("aria-checked"), "false");
    assert.equal(radios[1].getAttribute("aria-checked"), "true");
    assert.deepEqual(values, ["b"]);
  } finally {
    rendered.unmount();
  }
});
