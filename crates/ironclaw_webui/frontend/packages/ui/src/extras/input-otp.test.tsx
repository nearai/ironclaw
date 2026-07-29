// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React, { act } from "react";
import { renderIntoDocument } from "./test-helpers";
import { InputOTP } from "./input-otp";

function Harness({ length = 6, onComplete }: { length?: number; onComplete?: (v: string) => void }) {
  const [value, setValue] = React.useState("");
  return <InputOTP value={value} onChange={setValue} length={length} onComplete={onComplete} />;
}

function typeInto(input: HTMLInputElement, text: string) {
  act(() => {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
    setter?.call(input, text);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

test("InputOTP renders one labelled cell per digit", () => {
  const rendered = renderIntoDocument(<Harness length={4} />);
  try {
    const group = rendered.container.querySelector('[role="group"]');
    assert.equal(group?.getAttribute("aria-label"), "One-time code");
    const cells = rendered.container.querySelectorAll("input");
    assert.equal(cells.length, 4);
    assert.equal(cells[0].getAttribute("aria-label"), "Digit 1 of 4");
    assert.equal(cells[3].getAttribute("aria-label"), "Digit 4 of 4");
    assert.equal(cells[0].getAttribute("autocomplete"), "one-time-code");
  } finally {
    rendered.unmount();
  }
});

test("InputOTP accepts digits, strips non-digits, and fires onComplete", () => {
  const completed: string[] = [];
  const rendered = renderIntoDocument(
    <Harness length={3} onComplete={(code) => completed.push(code)} />
  );
  try {
    const cells = rendered.container.querySelectorAll<HTMLInputElement>("input");
    typeInto(cells[0], "4");
    typeInto(cells[1], "x"); // rejected: numeric mode
    assert.equal(cells[1].value, "");
    typeInto(cells[1], "2");
    typeInto(cells[2], "7");
    assert.deepEqual(completed, ["427"]);
    assert.equal(cells[0].value, "4");
    assert.equal(cells[2].value, "7");
  } finally {
    rendered.unmount();
  }
});
