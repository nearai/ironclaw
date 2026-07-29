// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { renderIntoDocument } from "./test-helpers";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "./resizable";

test("Resizable renders group, panels, and a keyboard-accessible separator", () => {
  const rendered = renderIntoDocument(
    <ResizablePanelGroup orientation="horizontal">
      <ResizablePanel defaultSize="30%">left</ResizablePanel>
      <ResizableHandle withHandle />
      <ResizablePanel>right</ResizablePanel>
    </ResizablePanelGroup>
  );
  try {
    assert.ok(rendered.container.querySelector("[data-group]"), "group root renders");
    assert.equal(rendered.container.querySelectorAll("[data-panel]").length, 2);
    const separator = rendered.container.querySelector('[role="separator"]');
    assert.ok(separator, "separator role for keyboard resizing");
    assert.match(rendered.container.textContent ?? "", /left/);
    assert.match(rendered.container.textContent ?? "", /right/);
  } finally {
    rendered.unmount();
  }
});
