import assert from "node:assert/strict";
import { isValidElement, type ReactElement } from "react";
import { test } from "vitest";

import { Button } from "./button";
import { Spinner } from "./spinner";

type ButtonElementProps = {
  className?: string;
  "aria-busy"?: boolean;
  children?: unknown;
  disabled?: boolean;
  "aria-disabled"?: boolean;
  href?: string;
  tabIndex?: number;
  to?: string;
  onClick?: (event: { preventDefault: () => void; stopPropagation: () => void }) => void;
};

function LinkLike() {
  return null;
}

function includesElementType(node: unknown, elementType: unknown): boolean {
  if (Array.isArray(node)) {
    return node.some((child) => includesElementType(child, elementType));
  }
  if (!isValidElement<{ children?: unknown }>(node)) return false;
  return node.type === elementType || includesElementType(node.props.children, elementType);
}

for (const variant of ["primary", "secondary"] as const) {
  test(`Button loading (${variant}) renders a spinner, disables, and sets aria-busy`, () => {
    const rendered = Button({
      variant,
      loading: true,
      children: "Connect",
    }) as ReactElement<ButtonElementProps>;

    assert.equal(rendered.props.disabled, true);
    assert.equal(rendered.props["aria-busy"], true);
    assert.ok(includesElementType(rendered.props.children, Spinner));
  });

  test(`Button idle (${variant}) has no spinner and is enabled`, () => {
    const rendered = Button({
      variant,
      loading: false,
      children: "Connect",
    }) as ReactElement<ButtonElementProps>;

    assert.equal(rendered.props.disabled, false);
    assert.equal(rendered.props["aria-busy"], undefined);
    assert.equal(includesElementType(rendered.props.children, Spinner), false);
  });

  test(`Button loading anchor (${variant}) blocks clicks and marks itself disabled`, () => {
    let clicked = false;
    let prevented = false;
    let stopped = false;
    const rendered = Button({
      as: "a",
      href: "https://example.com/auth",
      variant,
      loading: true,
      onClick: () => {
        clicked = true;
      },
      children: "Connect",
    }) as ReactElement<ButtonElementProps>;

    rendered.props.onClick?.({
      preventDefault: () => {
        prevented = true;
      },
      stopPropagation: () => {
        stopped = true;
      },
    });

    assert.equal(clicked, false);
    assert.equal(prevented, true);
    assert.equal(stopped, true);
    assert.equal(rendered.props.disabled, undefined);
    assert.equal(rendered.props["aria-disabled"], true);
    assert.equal(rendered.props.tabIndex, -1);
  });
}

test("disabled Link-like Buttons use aria-disabled without native disabled", () => {
  const rendered = Button({
    as: LinkLike,
    to: "/chat/thread-a",
    disabled: true,
    children: "Open run",
  }) as ReactElement<ButtonElementProps>;

  assert.equal(rendered.type, LinkLike);
  assert.equal(rendered.props.to, "/chat/thread-a");
  assert.equal(rendered.props.disabled, undefined);
  assert.equal(rendered.props["aria-disabled"], true);
  assert.equal(rendered.props.tabIndex, -1);

  let prevented = false;
  let stopped = false;
  rendered.props.onClick?.({
    preventDefault: () => {
      prevented = true;
    },
    stopPropagation: () => {
      stopped = true;
    },
  });
  assert.equal(prevented, true);
  assert.equal(stopped, true);
});

test("disabled native Buttons keep the disabled attribute", () => {
  const rendered = Button({
    disabled: true,
    children: "Save",
  }) as ReactElement<ButtonElementProps>;

  assert.equal(rendered.type, "button");
  assert.equal(rendered.props.disabled, true);
  assert.equal(rendered.props["aria-disabled"], undefined);
  const children = rendered.props.children as ReactElement[];
  const content = children[1] as ReactElement<{ children: [unknown, unknown] }>;
  assert.equal(content.props.children[0], false, "disabled without loading renders no spinner");
  assert.equal(content.props.children[1], "Save");
});

test("outline and danger Buttons use theme-aware semantic colors (#6039)", () => {
  const outline = Button({
    variant: "outline",
    children: "Configure",
  }) as ReactElement<ButtonElementProps>;
  const danger = Button({
    variant: "danger",
    children: "Remove",
  }) as ReactElement<ButtonElementProps>;

  assert.match(outline.props.className ?? "", /text-\[var\(--v2-accent-text\)\]/);
  assert.match(outline.props.className ?? "", /hover:bg-\[var\(--v2-accent-soft\)\]/);
  assert.match(danger.props.className ?? "", /text-\[var\(--v2-danger-text\)\]/);
  assert.match(danger.props.className ?? "", /hover:bg-\[var\(--v2-danger-soft\)\]/);
  assert.doesNotMatch(outline.props.className ?? "", /#8fc8f2|#4ca7e6/i);
  assert.doesNotMatch(danger.props.className ?? "", /#ff6480|rgba\(217,101,116/i);
});
