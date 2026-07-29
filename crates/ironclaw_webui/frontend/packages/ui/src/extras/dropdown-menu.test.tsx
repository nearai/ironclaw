// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { renderIntoDocument } from "./test-helpers";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "./dropdown-menu";

test("DropdownMenu renders menu semantics when open", () => {
  const rendered = renderIntoDocument(
    <DropdownMenu open>
      <DropdownMenuTrigger>Options</DropdownMenuTrigger>
      <DropdownMenuContent>
        <DropdownMenuLabel>Workspace</DropdownMenuLabel>
        <DropdownMenuItem>Rename</DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem tone="danger">Delete</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
  try {
    const trigger = rendered.container.querySelector("button");
    assert.equal(trigger?.getAttribute("aria-expanded"), "true");
    const menu = document.body.querySelector('[role="menu"]');
    assert.ok(menu, "menu content portals into the body");
    const items = menu.querySelectorAll('[role="menuitem"]');
    assert.equal(items.length, 2);
    assert.match(items[1].className, /--v2-danger-text/);
    assert.ok(menu.querySelector('[role="separator"]'));
  } finally {
    rendered.unmount();
  }
});
