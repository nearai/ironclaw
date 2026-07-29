// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { renderIntoDocument } from "./test-helpers";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "./context-menu";

test("ContextMenu renders its trigger area with the Radix data hook", () => {
  const rendered = renderIntoDocument(
    <ContextMenu>
      <ContextMenuTrigger>Right-click zone</ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem>Copy</ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
  try {
    const trigger = rendered.container.querySelector("[data-state]");
    assert.ok(trigger, "trigger carries a data-state attribute");
    assert.equal(trigger.getAttribute("data-state"), "closed");
    assert.match(rendered.container.textContent ?? "", /Right-click zone/);
    // Menu content stays out of the tree until the contextmenu gesture.
    assert.equal(document.body.querySelector('[role="menu"]'), null);
  } finally {
    rendered.unmount();
  }
});
