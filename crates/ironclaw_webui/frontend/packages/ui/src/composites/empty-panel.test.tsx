import assert from "node:assert/strict";
import { type ReactElement } from "react";
import { test } from "vitest";

import { EmptyPanel } from "./empty-panel";
import { Card } from "../components/card";

type PanelProps = { className?: string; children?: unknown };

test("EmptyPanel default variant wraps the body in a Card", () => {
  const rendered = EmptyPanel({ title: "Nothing here" }) as ReactElement<PanelProps>;
  assert.equal(rendered.type, Card);
});

test("EmptyPanel plain variant renders without Card chrome", () => {
  const rendered = EmptyPanel({ variant: "plain", title: "Nothing here" }) as ReactElement<PanelProps>;
  assert.equal(rendered.type, "div");
  assert.match(JSON.stringify(rendered), /Nothing here/);
});

test("EmptyPanel dashed variant renders the compact drop-zone placeholder", () => {
  const rendered = EmptyPanel({
    variant: "dashed",
    description: "No missions yet.",
  }) as ReactElement<PanelProps>;
  assert.equal(rendered.type, "div");
  assert.match(rendered.props.className ?? "", /border-dashed/);
  assert.match(JSON.stringify(rendered), /No missions yet\./);
});
