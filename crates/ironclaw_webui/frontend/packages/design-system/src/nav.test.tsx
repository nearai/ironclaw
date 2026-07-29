import assert from "node:assert/strict";
import type { ReactElement } from "react";
import { test } from "vitest";

import { NavItem, NavList } from "./nav";

type ElementProps = {
  "aria-current"?: string;
  "aria-label"?: string;
  className?: string;
  href?: string;
  type?: string;
  children?: unknown;
};

test("NavList renders a labelled nav region", () => {
  const rendered = NavList({ label: "Primary", children: null }) as ReactElement<ElementProps>;
  assert.equal(rendered.type, "nav");
  assert.equal(rendered.props["aria-label"], "Primary");
});

test("NavItem defaults to a type=button and marks active with aria-current", () => {
  const idle = NavItem({ label: "Runs" }) as ReactElement<ElementProps>;
  assert.equal(idle.type, "button");
  assert.equal(idle.props.type, "button");
  assert.equal(idle.props["aria-current"], undefined);

  const active = NavItem({ label: "Runs", active: true }) as ReactElement<ElementProps>;
  assert.equal(active.props["aria-current"], "page");
  assert.match(active.props.className ?? "", /text-\[var\(--v2-text-strong\)\]/);
});

test("NavItem renders link-like elements without a button type", () => {
  const rendered = NavItem({
    label: "Docs",
    as: "a",
    href: "/docs",
  }) as ReactElement<ElementProps>;
  assert.equal(rendered.type, "a");
  assert.equal(rendered.props.href, "/docs");
  assert.equal(rendered.props.type, undefined);
});
