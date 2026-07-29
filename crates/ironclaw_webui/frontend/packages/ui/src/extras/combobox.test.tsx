// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React, { act } from "react";
import { click, renderIntoDocument } from "./test-helpers";
import { Combobox } from "./combobox";

const OPTIONS = [
  { value: "us-east", label: "US East" },
  { value: "eu-central", label: "EU Central" },
  { value: "ap-south", label: "Asia Pacific" },
];

test("Combobox opens a listbox and filters options by the query", () => {
  const rendered = renderIntoDocument(
    <Combobox options={OPTIONS} value="us-east" aria-label="Region" />
  );
  try {
    const trigger = rendered.container.querySelector("button");
    assert.ok(trigger);
    assert.equal(trigger.getAttribute("aria-haspopup"), "listbox");
    assert.match(trigger.textContent ?? "", /US East/);

    click(trigger);
    const input = rendered.container.querySelector<HTMLInputElement>('input[role="combobox"]');
    assert.ok(input, "search input appears when open");
    assert.equal(rendered.container.querySelectorAll('[role="option"]').length, 3);

    // Filter down to the EU option (React 19 needs the native setter to
    // trigger onChange through happy-dom).
    act(() => {
      const setter = Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        "value"
      )?.set;
      setter?.call(input, "eu");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    const options = rendered.container.querySelectorAll('[role="option"]');
    assert.equal(options.length, 1);
    assert.match(options[0].textContent ?? "", /EU Central/);
  } finally {
    rendered.unmount();
  }
});

test("Combobox selects an option on click and closes", () => {
  const chosen: string[] = [];
  const rendered = renderIntoDocument(
    <Combobox options={OPTIONS} onChange={(value) => chosen.push(value)} aria-label="Region" />
  );
  try {
    click(rendered.container.querySelector("button")!);
    const option = Array.from(
      rendered.container.querySelectorAll('[role="option"]')
    ).find((node) => /EU Central/.test(node.textContent ?? ""));
    assert.ok(option);
    click(option);
    assert.deepEqual(chosen, ["eu-central"]);
    assert.equal(rendered.container.querySelector('[role="listbox"]'), null);
  } finally {
    rendered.unmount();
  }
});
