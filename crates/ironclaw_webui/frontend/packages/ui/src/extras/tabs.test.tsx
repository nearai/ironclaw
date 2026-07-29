// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { fire, renderIntoDocument } from "./test-helpers";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "./tabs";

test("Tabs render tablist semantics and switch panels on click", () => {
  const rendered = renderIntoDocument(
    <Tabs defaultValue="overview">
      <TabsList>
        <TabsTrigger value="overview">Overview</TabsTrigger>
        <TabsTrigger value="logs">Logs</TabsTrigger>
      </TabsList>
      <TabsContent value="overview">Overview panel</TabsContent>
      <TabsContent value="logs">Logs panel</TabsContent>
    </Tabs>
  );
  try {
    assert.ok(rendered.container.querySelector('[role="tablist"]'));
    const tabs = rendered.container.querySelectorAll('[role="tab"]');
    assert.equal(tabs.length, 2);
    assert.equal(tabs[0].getAttribute("aria-selected"), "true");
    assert.match(rendered.container.textContent ?? "", /Overview panel/);
    assert.doesNotMatch(rendered.container.textContent ?? "", /Logs panel/);

    // Radix Tabs activate on mousedown (not click).
    fire(tabs[1], new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    assert.equal(tabs[1].getAttribute("aria-selected"), "true");
    assert.match(rendered.container.textContent ?? "", /Logs panel/);
    assert.ok(rendered.container.querySelector('[role="tabpanel"]'));
  } finally {
    rendered.unmount();
  }
});
