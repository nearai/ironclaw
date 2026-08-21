import assert from "node:assert/strict";
import { isValidElement, type ReactElement } from "react";
import { test } from "vitest";

import { PageScroll, PageStack } from "./page-shell";

type ElementProps = {
  children?: unknown;
  className?: string;
};

test("PageScroll owns the standard page scroller and responsive content padding", () => {
  const rendered = PageScroll({ children: "content" }) as ReactElement<ElementProps>;
  const content = rendered.props.children as ReactElement<ElementProps>;

  assert.equal(rendered.type, "div");
  assert.match(rendered.props.className ?? "", /\bflex h-full flex-col overflow-y-auto\b/);
  assert.equal(content.type, "div");
  assert.match(content.props.className ?? "", /\bv2-page-entrance flex-1 p-4 sm:p-6\b/);
  assert.equal(content.props.children, "content");
});

test("PageScroll preserves the settings-style contained scroll owner", () => {
  const rendered = PageScroll({ children: "content", contained: true }) as ReactElement<ElementProps>;
  const scroller = rendered.props.children as ReactElement<ElementProps>;
  const content = scroller.props.children as ReactElement<ElementProps>;

  assert.match(rendered.props.className ?? "", /\bflex h-full min-h-0 flex-col overflow-hidden\b/);
  assert.match(scroller.props.className ?? "", /\bmin-h-0 flex-1 overflow-y-auto\b/);
  assert.match(content.props.className ?? "", /\bv2-page-entrance flex-1 p-4 sm:p-6\b/);
});

test("PageScroll applies scroll constraints to the full-page scroll owner", () => {
  const rendered = PageScroll({
    children: "content",
    scrollClassName: "overscroll-contain scroll-smooth",
  }) as ReactElement<ElementProps>;

  assert.match(rendered.props.className ?? "", /\boverscroll-contain\b/);
  assert.match(rendered.props.className ?? "", /\bscroll-smooth\b/);
});

test("PageScroll and PageStack append page-specific constraints", () => {
  const scroll = PageScroll({
    children: "content",
    className: "page-shell",
    contentClassName: "page-content",
  }) as ReactElement<ElementProps>;
  const content = scroll.props.children as ReactElement<ElementProps>;
  const stack = PageStack({
    children: "stacked",
    className: "flex h-full min-h-0 flex-col",
  }) as ReactElement<ElementProps>;

  assert.match(scroll.props.className ?? "", /\bpage-shell\b/);
  assert.match(content.props.className ?? "", /\bpage-content\b/);
  assert.equal(stack.type, "div");
  assert.match(stack.props.className ?? "", /\bspace-y-5\b/);
  assert.match(stack.props.className ?? "", /\bflex h-full min-h-0 flex-col\b/);
  assert.equal(stack.props.children, "stacked");
  assert.equal(isValidElement(stack), true);
});

test("PageScroll keeps overlays outside the padded entrance content", () => {
  const overlay = <div data-testid="overlay" />;
  const rendered = PageScroll({ children: "content", overlay }) as ReactElement<ElementProps>;
  const page = rendered.props.children as ReactElement<ElementProps>;
  const [content, renderedOverlay] = page.props.children as ReactElement<ElementProps>[];
  const contained = PageScroll({
    children: "content",
    contained: true,
    overlay,
  }) as ReactElement<ElementProps>;
  const containedPage = contained.props.children as ReactElement<ElementProps>;
  const [scroller, containedOverlay] = containedPage.props.children as ReactElement<ElementProps>[];

  assert.equal(content.props.children, "content");
  assert.equal(renderedOverlay, overlay);
  assert.match(scroller.props.className ?? "", /\boverflow-y-auto\b/);
  assert.equal(containedOverlay, overlay);
});
