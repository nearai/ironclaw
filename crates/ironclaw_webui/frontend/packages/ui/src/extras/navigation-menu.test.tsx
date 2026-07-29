// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { renderIntoDocument } from "./test-helpers";
import {
  NavigationMenu,
  NavigationMenuContent,
  NavigationMenuItem,
  NavigationMenuLink,
  NavigationMenuList,
  NavigationMenuTrigger,
} from "./navigation-menu";

test("NavigationMenu renders nav landmark, triggers, and plain links", () => {
  const rendered = renderIntoDocument(
    <NavigationMenu>
      <NavigationMenuList>
        <NavigationMenuItem>
          <NavigationMenuTrigger>Product</NavigationMenuTrigger>
          <NavigationMenuContent>
            <NavigationMenuLink href="#agents">Agents</NavigationMenuLink>
          </NavigationMenuContent>
        </NavigationMenuItem>
        <NavigationMenuItem>
          <NavigationMenuLink href="#docs">Docs</NavigationMenuLink>
        </NavigationMenuItem>
      </NavigationMenuList>
    </NavigationMenu>
  );
  try {
    assert.ok(rendered.container.querySelector("nav"), "renders a nav landmark");
    const trigger = rendered.container.querySelector("button");
    assert.ok(trigger);
    assert.match(trigger.textContent ?? "", /Product/);
    assert.equal(trigger.getAttribute("aria-expanded"), "false");
    const link = rendered.container.querySelector('a[href="#docs"]');
    assert.ok(link, "plain links render as anchors");
  } finally {
    rendered.unmount();
  }
});
