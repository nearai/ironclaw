import assert from "node:assert/strict";
import { isValidElement, type ReactElement } from "react";
import { test } from "vitest";

import { Icon } from "./icons";
import { ToolIcon } from "./tool-icon";

type ElementProps = {
  role?: string;
  "aria-label"?: string;
  className?: string;
  children?: unknown;
};

test("ToolIcon resolves known tools to a system glyph", () => {
  const rendered = ToolIcon({ name: "GitHub" }) as ReactElement<ElementProps>;
  assert.equal(rendered.props.role, "img");
  assert.equal(rendered.props["aria-label"], "GitHub");
  const child = rendered.props.children;
  assert.ok(isValidElement(child) && child.type === Icon);
});

test("ToolIcon falls back to a monogram for unknown tools", () => {
  const rendered = ToolIcon({ name: "Stripe" }) as ReactElement<ElementProps>;
  const child = rendered.props.children as ReactElement<{ children?: unknown }>;
  assert.ok(isValidElement(child) && child.type === "span");
  assert.equal(child.props.children, "S");
});

test("ToolIcon icon override wins over the registry", () => {
  const rendered = ToolIcon({ name: "Gmail", icon: "bolt" }) as ReactElement<ElementProps>;
  const child = rendered.props.children as ReactElement<{ name?: string }>;
  assert.ok(isValidElement(child) && child.type === Icon);
  assert.equal(child.props.name, "bolt");
});
