// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { renderIntoDocument } from "./test-helpers";
import { Avatar, AvatarFallback, AvatarImage } from "./avatar";

test("Avatar shows the fallback while the image has not loaded", () => {
  const rendered = renderIntoDocument(
    <Avatar>
      <AvatarImage src="https://example.com/a.png" alt="Ada" />
      <AvatarFallback>AL</AvatarFallback>
    </Avatar>
  );
  try {
    assert.match(rendered.container.textContent ?? "", /AL/);
  } finally {
    rendered.unmount();
  }
});

test("Avatar sizes map to the control scale", () => {
  const rendered = renderIntoDocument(
    <Avatar size="lg">
      <AvatarFallback>L</AvatarFallback>
    </Avatar>
  );
  try {
    const rootSpan = rendered.container.querySelector("span");
    assert.match(rootSpan?.className ?? "", /h-12/);
  } finally {
    rendered.unmount();
  }
});
