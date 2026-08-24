import assert from "node:assert/strict";
import { type ReactElement } from "react";
import { test } from "vitest";

import { Skeleton, SkeletonList } from "./skeleton";

type ElementProps = {
  "aria-hidden"?: boolean;
  "aria-label"?: string;
  children?: ReactElement[];
  className?: string;
  role?: string;
};

test("Skeleton applies the shared styling class and stays decorative by default", () => {
  const rendered = Skeleton({ className: "h-8 rounded-md" }) as ReactElement<ElementProps>;

  assert.equal(rendered.type, "div");
  assert.match(rendered.props.className ?? "", /\bv2-skeleton\b/);
  assert.match(rendered.props.className ?? "", /\bh-8 rounded-md\b/);
  assert.equal(rendered.props["aria-hidden"], true);
});

test("Skeleton keeps a supplied loading label available to assistive technology", () => {
  const rendered = Skeleton({
    "aria-label": "Loading configuration",
    className: "h-48 rounded-xl",
  }) as ReactElement<ElementProps>;

  assert.equal(rendered.props["aria-label"], "Loading configuration");
  assert.equal(rendered.props["aria-hidden"], undefined);
  assert.equal(rendered.props.role, "status");
});

test("SkeletonList renders the requested number of consistently shaped placeholders", () => {
  const rendered = SkeletonList({
    count: 3,
    className: "space-y-2",
    itemClassName: "h-12 rounded-lg",
  }) as ReactElement<ElementProps>;
  const items = (rendered.props.children ?? []) as ReactElement<{ className?: string }>[];

  assert.equal(rendered.type, "div");
  assert.equal(rendered.props.className, "space-y-2");
  assert.equal(items.length, 3);
  for (const item of items) {
    assert.equal(item.type, Skeleton);
    assert.equal(item.props.className, "h-12 rounded-lg");
  }
});

test("SkeletonList exposes a supplied loading label as a status", () => {
  const rendered = SkeletonList({
    "aria-label": "Loading extensions",
    count: 2,
  }) as ReactElement<ElementProps>;

  assert.equal(rendered.props["aria-label"], "Loading extensions");
  assert.equal(rendered.props.role, "status");
});
