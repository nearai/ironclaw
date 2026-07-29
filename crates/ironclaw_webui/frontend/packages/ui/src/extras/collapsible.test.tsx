// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { click, renderIntoDocument } from "./test-helpers";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "./collapsible";

test("Collapsible hides content until opened", () => {
  const rendered = renderIntoDocument(
    <Collapsible>
      <CollapsibleTrigger>Toggle</CollapsibleTrigger>
      <CollapsibleContent>Hidden details</CollapsibleContent>
    </Collapsible>
  );
  try {
    const trigger = rendered.container.querySelector("button");
    assert.ok(trigger);
    assert.equal(trigger.getAttribute("aria-expanded"), "false");
    assert.doesNotMatch(rendered.container.textContent ?? "", /Hidden details/);
    click(trigger);
    assert.equal(trigger.getAttribute("aria-expanded"), "true");
    assert.match(rendered.container.textContent ?? "", /Hidden details/);
  } finally {
    rendered.unmount();
  }
});
