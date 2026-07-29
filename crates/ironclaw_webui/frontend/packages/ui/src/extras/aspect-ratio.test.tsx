import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { AspectRatio } from "./aspect-ratio";

test("AspectRatio renders its child inside the ratio wrapper", () => {
  const html = renderToStaticMarkup(
    <AspectRatio ratio={16 / 9}>
      <span>framed</span>
    </AspectRatio>
  );
  assert.match(html, /framed/);
  // Radix implements the ratio via padding-bottom on the outer wrapper.
  assert.match(html, /padding-bottom/);
});
