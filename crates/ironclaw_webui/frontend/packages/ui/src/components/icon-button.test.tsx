// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
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

test("the polymorphic contract types refs against the rendered element (type-level)", () => {
  const buttonRef = React.createRef<HTMLButtonElement>();
  const anchorRef = React.createRef<HTMLAnchorElement>();

  void (<IconButton ref={buttonRef} aria-label="Bell" />);
  void (<IconButton as="a" href="/inbox" ref={anchorRef} aria-label="Inbox" />);

  // @ts-expect-error — an anchor ref is not assignable to the default button
  void (<IconButton ref={anchorRef} aria-label="Bell" />);
  // @ts-expect-error — a button ref is not assignable when rendering an anchor
  void (<IconButton as="a" href="/inbox" ref={buttonRef} aria-label="Inbox" />);

  assert.ok(true);
});

test("refs attach to the underlying element for button and anchor renders", () => {
  const container = document.createElement("div");
  document.body.append(container);
  const buttonRef = React.createRef<HTMLButtonElement>();
  const anchorRef = React.createRef<HTMLAnchorElement>();

  const root = createRoot(container);
  try {
    act(() =>
      root.render(
        <>
          <IconButton ref={buttonRef} aria-label="Bell" />
          <IconButton as="a" href="/inbox" ref={anchorRef} aria-label="Inbox" />
        </>
      )
    );
    assert.ok(buttonRef.current instanceof HTMLButtonElement);
    assert.equal(buttonRef.current.type, "button");
    assert.ok(anchorRef.current instanceof HTMLAnchorElement);
  } finally {
    act(() => root.unmount());
    container.remove();
  }
});
