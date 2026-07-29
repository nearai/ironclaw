// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { renderIntoDocument } from "./test-helpers";
import { Menubar, MenubarContent, MenubarItem, MenubarMenu, MenubarTrigger } from "./menubar";

test("Menubar renders menubar semantics with menuitem triggers", () => {
  const rendered = renderIntoDocument(
    <Menubar>
      <MenubarMenu>
        <MenubarTrigger>File</MenubarTrigger>
        <MenubarContent>
          <MenubarItem>New run</MenubarItem>
        </MenubarContent>
      </MenubarMenu>
      <MenubarMenu>
        <MenubarTrigger>View</MenubarTrigger>
        <MenubarContent>
          <MenubarItem>Zoom</MenubarItem>
        </MenubarContent>
      </MenubarMenu>
    </Menubar>
  );
  try {
    const bar = rendered.container.querySelector('[role="menubar"]');
    assert.ok(bar);
    const triggers = bar.querySelectorAll('[role="menuitem"]');
    assert.equal(triggers.length, 2);
    assert.equal(triggers[0].getAttribute("aria-haspopup"), "menu");
    assert.equal(triggers[0].getAttribute("data-state"), "closed");
  } finally {
    rendered.unmount();
  }
});
