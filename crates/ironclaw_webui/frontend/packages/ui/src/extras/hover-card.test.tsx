// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { renderIntoDocument } from "./test-helpers";
import { HoverCard, HoverCardContent, HoverCardTrigger } from "./hover-card";

test("HoverCard shows its content when open", () => {
  const rendered = renderIntoDocument(
    <HoverCard open>
      <HoverCardTrigger>@agent</HoverCardTrigger>
      <HoverCardContent>Profile preview</HoverCardContent>
    </HoverCard>
  );
  try {
    assert.match(rendered.container.textContent ?? "", /@agent/);
    assert.match(document.body.textContent ?? "", /Profile preview/);
  } finally {
    rendered.unmount();
  }
});
