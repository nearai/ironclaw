import assert from "node:assert/strict";
import type { ReactElement } from "react";
import { test } from "vitest";

import { Callout } from "./callout";

type ElementProps = {
  role?: string;
  className?: string;
  children?: unknown;
};

test("Callout renders an aside with role=note", () => {
  const rendered = Callout({ children: "Body" }) as ReactElement<ElementProps>;
  assert.equal(rendered.type, "aside");
  assert.equal(rendered.props.role, "note");
});

test("Callout tones map to semantic token surfaces", () => {
  const info = Callout({ tone: "info", children: "x" }) as ReactElement<ElementProps>;
  assert.match(info.props.className ?? "", /--v2-info-soft/);
  const danger = Callout({ tone: "danger", children: "x" }) as ReactElement<ElementProps>;
  assert.match(danger.props.className ?? "", /--v2-danger-soft/);
});

test("Callout without an icon collapses to a single column", () => {
  const rendered = Callout({ icon: null, children: "x" }) as ReactElement<ElementProps>;
  assert.match(rendered.props.className ?? "", /grid-cols-1/);
});
