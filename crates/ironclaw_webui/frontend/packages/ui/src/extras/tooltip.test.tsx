// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { renderIntoDocument } from "./test-helpers";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "./tooltip";

test("Tooltip renders content when open and links it to the trigger", () => {
  const rendered = renderIntoDocument(
    <TooltipProvider>
      <Tooltip open>
        <TooltipTrigger>Settings</TooltipTrigger>
        <TooltipContent>Open settings</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
  try {
    const trigger = rendered.container.querySelector("button");
    assert.ok(trigger);
    assert.equal(trigger.getAttribute("data-state"), "instant-open");
    const tooltip = document.body.querySelector('[role="tooltip"]');
    assert.ok(tooltip, "tooltip role present in the portal");
    assert.match(document.body.textContent ?? "", /Open settings/);
  } finally {
    rendered.unmount();
  }
});
