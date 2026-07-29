// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { renderIntoDocument } from "./test-helpers";
import { Popover, PopoverContent, PopoverTrigger } from "./popover";

test("Popover portals dialog content into the body when open", () => {
  const rendered = renderIntoDocument(
    <Popover open>
      <PopoverTrigger>Filters</PopoverTrigger>
      <PopoverContent>Popover body</PopoverContent>
    </Popover>
  );
  try {
    const trigger = rendered.container.querySelector("button");
    assert.equal(trigger?.getAttribute("aria-expanded"), "true");
    const content = document.body.querySelector('[role="dialog"]');
    assert.ok(content, "content renders with dialog role");
    assert.match(content.textContent ?? "", /Popover body/);
  } finally {
    rendered.unmount();
  }
});
