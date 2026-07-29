import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Progress } from "./progress";

test("Progress renders progressbar semantics and a clamped indicator width", () => {
  const html = renderToStaticMarkup(<Progress value={64} aria-label="Upload" />);
  assert.match(html, /role="progressbar"/);
  assert.match(html, /aria-valuenow="64"/);
  assert.match(html, /width:64%/);
});

test("Progress tones map to semantic tokens", () => {
  const html = renderToStaticMarkup(<Progress value={30} tone="danger" aria-label="x" />);
  assert.match(html, /--v2-danger-text/);
  const clamped = renderToStaticMarkup(<Progress value={100} tone="positive" aria-label="y" />);
  assert.match(clamped, /--v2-positive-text/);
  assert.match(clamped, /width:100%/);
});
