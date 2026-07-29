// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React, { act } from "react";
import { renderIntoDocument } from "./test-helpers";
import { Drawer, DrawerBody, DrawerFooter, Sheet } from "./drawer";

test("Drawer renders a dialog when open and nothing when closed", () => {
  const closedRender = renderIntoDocument(
    <Drawer open={false} title="Details">
      <DrawerBody>Body</DrawerBody>
    </Drawer>
  );
  try {
    assert.equal(closedRender.container.querySelector('[role="dialog"]'), null);
  } finally {
    closedRender.unmount();
  }

  const rendered = renderIntoDocument(
    <Drawer open onClose={() => {}} title="Details" side="left">
      <DrawerBody>Body copy</DrawerBody>
      <DrawerFooter>Actions</DrawerFooter>
    </Drawer>
  );
  try {
    const dialog = rendered.container.querySelector('[role="dialog"]');
    assert.ok(dialog);
    assert.equal(dialog.getAttribute("aria-modal"), "true");
    assert.equal(dialog.getAttribute("aria-label"), "Details");
    assert.match(rendered.container.textContent ?? "", /Body copy/);
    assert.match(rendered.container.textContent ?? "", /Actions/);
  } finally {
    rendered.unmount();
  }
});

test("Drawer closes on Escape and exposes the Sheet alias", () => {
  assert.equal(Sheet, Drawer);
  let closed = 0;
  const rendered = renderIntoDocument(
    <Drawer open onClose={() => { closed += 1; }} title="Filters">
      <DrawerBody>content</DrawerBody>
    </Drawer>
  );
  try {
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    assert.equal(closed, 1);
  } finally {
    rendered.unmount();
  }
});
