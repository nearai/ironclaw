import assert from "node:assert/strict";
import type { ReactElement } from "react";
import { test } from "vitest";

import { ListRow } from "./list";

type ElementProps = {
  className?: string;
  type?: string;
  onClick?: () => void;
};

test("ListRow is a static div unless onClick is passed", () => {
  const rendered = ListRow({ title: "Morning digest" }) as ReactElement<ElementProps>;
  assert.equal(rendered.type, "div");
  assert.equal(rendered.props.type, undefined);
});

test("ListRow becomes an accessible button when interactive", () => {
  let clicked = false;
  const rendered = ListRow({
    title: "Morning digest",
    onClick: () => {
      clicked = true;
    },
  }) as ReactElement<ElementProps>;
  assert.equal(rendered.type, "button");
  assert.equal(rendered.props.type, "button");
  rendered.props.onClick?.();
  assert.equal(clicked, true);
  assert.match(rendered.props.className ?? "", /hover:bg-\[var\(--v2-surface-soft\)\]/);
});

test("ListRow divider is on by default and removable", () => {
  const divided = ListRow({ title: "A" }) as ReactElement<ElementProps>;
  assert.match(divided.props.className ?? "", /border-b/);
  const flush = ListRow({ title: "A", divider: false }) as ReactElement<ElementProps>;
  assert.doesNotMatch(flush.props.className ?? "", /border-b/);
});
