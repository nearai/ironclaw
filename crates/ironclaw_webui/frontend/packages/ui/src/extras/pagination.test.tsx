// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { click, renderIntoDocument } from "./test-helpers";
import { SimplePagination } from "./pagination";

test("SimplePagination renders a nav with aria-current on the active page", () => {
  const rendered = renderIntoDocument(
    <SimplePagination page={7} pageCount={20} onPageChange={() => {}} />
  );
  try {
    const nav = rendered.container.querySelector("nav");
    assert.equal(nav?.getAttribute("aria-label"), "Pagination");
    const current = rendered.container.querySelector('[aria-current="page"]');
    assert.match(current?.textContent ?? "", /^7$/);
    // Windowing: 1 … 6 7 8 … 20 plus prev/next.
    const text = rendered.container.textContent ?? "";
    assert.match(text, /1.*6.*7.*8.*20/s);
    assert.equal(rendered.container.querySelectorAll('span[aria-hidden="true"]').length, 2);
  } finally {
    rendered.unmount();
  }
});

test("SimplePagination fires onPageChange and disables edges", () => {
  const pages: number[] = [];
  const rendered = renderIntoDocument(
    <SimplePagination page={1} pageCount={3} onPageChange={(page) => pages.push(page)} />
  );
  try {
    const buttons = Array.from(rendered.container.querySelectorAll("button"));
    const previous = buttons[0];
    assert.equal(previous.disabled, true, "Previous disabled on first page");
    const pageTwo = buttons.find((button) => button.textContent === "2");
    assert.ok(pageTwo);
    click(pageTwo);
    const next = buttons[buttons.length - 1];
    click(next);
    assert.deepEqual(pages, [2, 2]);
  } finally {
    rendered.unmount();
  }
});
