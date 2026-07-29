// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { renderIntoDocument } from "./test-helpers";
import { ScrollArea } from "./scroll-area";

test("ScrollArea wraps content in a viewport", () => {
  const rendered = renderIntoDocument(
    <ScrollArea className="h-40">
      <div>row one</div>
      <div>row two</div>
    </ScrollArea>
  );
  try {
    assert.match(rendered.container.textContent ?? "", /row one/);
    assert.match(rendered.container.textContent ?? "", /row two/);
    assert.ok(
      rendered.container.querySelector("[data-radix-scroll-area-viewport]"),
      "radix viewport wrapper present"
    );
  } finally {
    rendered.unmount();
  }
});
