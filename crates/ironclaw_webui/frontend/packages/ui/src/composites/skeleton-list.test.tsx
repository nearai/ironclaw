import assert from "node:assert/strict";
import { Children, type ReactElement } from "react";
import { test } from "vitest";

import { SkeletonList } from "./skeleton-list";

type ListProps = {
  role?: string;
  "aria-label"?: string;
  children?: unknown;
};

test("SkeletonList renders the requested number of placeholder rows", () => {
  const rendered = SkeletonList({ count: 5 }) as ReactElement<ListProps>;
  assert.equal(Children.count(rendered.props.children), 5);
  assert.equal(rendered.props.role, undefined);
});

test("SkeletonList label adds a status live region", () => {
  const rendered = SkeletonList({ label: "Loading jobs" }) as ReactElement<ListProps>;
  assert.equal(rendered.props.role, "status");
  assert.equal(rendered.props["aria-label"], "Loading jobs");
  assert.equal(Children.count(rendered.props.children), 3);
});
