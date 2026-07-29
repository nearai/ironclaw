import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Separator } from "./separator";

test("Separator is decorative by default and semantic when asked", () => {
  const decorative = renderToStaticMarkup(<Separator />);
  assert.match(decorative, /role="none"/);
  assert.match(decorative, /--v2-panel-border/);

  const semantic = renderToStaticMarkup(<Separator decorative={false} orientation="vertical" />);
  assert.match(semantic, /role="separator"/);
  assert.match(semantic, /aria-orientation="vertical"/);
  assert.match(semantic, /w-px/);
});
