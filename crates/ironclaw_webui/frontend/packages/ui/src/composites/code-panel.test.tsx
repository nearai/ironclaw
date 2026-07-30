import assert from "node:assert/strict";
import { type ReactElement } from "react";
import { test } from "vitest";

import { CodePanel } from "./code-panel";

type PreProps = { className?: string; children?: unknown };

test("CodePanel scrolls horizontally by default", () => {
  const rendered = CodePanel({ children: "payload" }) as ReactElement<PreProps>;
  assert.equal(rendered.type, "pre");
  assert.match(rendered.props.className ?? "", /overflow-x-auto/);
});

test("CodePanel wrap soft-wraps long tokens instead of scrolling", () => {
  const rendered = CodePanel({ wrap: true, children: "payload" }) as ReactElement<PreProps>;
  assert.match(rendered.props.className ?? "", /whitespace-pre-wrap/);
  assert.doesNotMatch(rendered.props.className ?? "", /overflow-x-auto/);
});
