import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { IconButton, iconButtonClasses } from "./icon-button";

test("renders an inert button by default and honors an explicit type", () => {
  assert.match(renderToStaticMarkup(<IconButton aria-label="Open" />), /type="button"/);
  assert.match(
    renderToStaticMarkup(<IconButton type="submit" aria-label="Save" />),
    /type="submit"/
  );
});

test("renders anchors and Link-like components via `as`", () => {
  const anchor = renderToStaticMarkup(<IconButton as="a" href="/inbox" aria-label="Inbox" />);
  assert.match(anchor, /^<a /);
  assert.match(anchor, /href="\/inbox"/);
  assert.doesNotMatch(anchor, /type=/);

  function FakeLink({
    to,
    className,
    children,
  }: {
    to: string;
    className?: string;
    children?: React.ReactNode;
  }) {
    return (
      <a data-link="true" href={to} className={className}>
        {children}
      </a>
    );
  }
  const link = renderToStaticMarkup(<IconButton as={FakeLink} to="/jobs" />);
  assert.match(link, /data-link/);
  assert.match(link, /href="\/jobs"/);
});

test("active styling matches the exported class builder", () => {
  const markup = renderToStaticMarkup(<IconButton active aria-label="Bell" />);
  assert.ok(markup.includes(iconButtonClasses({ active: true })));
});

test("the polymorphic contract rejects props invalid for the element (type-level)", () => {
  // @ts-expect-error — href is not a valid attribute of the default button
  void (<IconButton href="/x" />);

  function FakeLink({ to }: { to: string }) {
    return <a href={to} />;
  }
  // @ts-expect-error — FakeLink requires its `to` prop
  void (<IconButton as={FakeLink} />);

  assert.ok(true);
});
