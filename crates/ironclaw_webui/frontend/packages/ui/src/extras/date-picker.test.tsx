// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { click, renderIntoDocument } from "./test-helpers";
import { Calendar, DatePicker } from "./date-picker";

test("Calendar renders a grid for the month and selects a day", () => {
  const chosen: Date[] = [];
  const rendered = renderIntoDocument(
    <Calendar
      value={new Date(2026, 6, 15)}
      onChange={(date) => chosen.push(date)}
    />
  );
  try {
    assert.ok(rendered.container.querySelector('[role="grid"]'));
    assert.match(rendered.container.textContent ?? "", /July 2026/);
    assert.equal(rendered.container.querySelectorAll("th").length, 7);

    const selectedCell = rendered.container.querySelector('[role="gridcell"][aria-selected="true"]');
    assert.ok(selectedCell, "selected day is marked on the gridcell");
    assert.match(selectedCell.textContent ?? "", /15/);

    const day20 = rendered.container.querySelector('button[data-date="2026-07-20"]');
    assert.ok(day20);
    click(day20);
    assert.equal(chosen.length, 1);
    assert.equal(chosen[0].getDate(), 20);
    assert.equal(chosen[0].getMonth(), 6);
  } finally {
    rendered.unmount();
  }
});

test("Calendar month navigation moves the visible month", () => {
  const rendered = renderIntoDocument(<Calendar defaultMonth={new Date(2026, 0, 10)} />);
  try {
    click(rendered.container.querySelector('button[aria-label="Next month"]')!);
    assert.match(rendered.container.textContent ?? "", /February 2026/);
    click(rendered.container.querySelector('button[aria-label="Previous month"]')!);
    assert.match(rendered.container.textContent ?? "", /January 2026/);
  } finally {
    rendered.unmount();
  }
});

test("Calendar disables days outside min/max bounds", () => {
  const rendered = renderIntoDocument(
    <Calendar
      defaultMonth={new Date(2026, 6, 1)}
      minDate={new Date(2026, 6, 10)}
      maxDate={new Date(2026, 6, 20)}
    />
  );
  try {
    const day5 = rendered.container.querySelector<HTMLButtonElement>('button[data-date="2026-07-05"]');
    const day15 = rendered.container.querySelector<HTMLButtonElement>('button[data-date="2026-07-15"]');
    assert.equal(day5?.disabled, true);
    assert.equal(day15?.disabled, false);
  } finally {
    rendered.unmount();
  }
});

test("DatePicker opens a calendar dialog and closes after selection", () => {
  const chosen: Date[] = [];
  const rendered = renderIntoDocument(
    <DatePicker
      value={new Date(2026, 6, 15)}
      onChange={(date) => chosen.push(date)}
      aria-label="Due date"
    />
  );
  try {
    const trigger = rendered.container.querySelector("button");
    assert.ok(trigger);
    assert.equal(trigger.getAttribute("aria-haspopup"), "dialog");
    click(trigger);
    assert.ok(rendered.container.querySelector('[role="dialog"]'));
    click(rendered.container.querySelector('button[data-date="2026-07-21"]')!);
    assert.equal(chosen.length, 1);
    assert.equal(rendered.container.querySelector('[role="dialog"]'), null);
  } finally {
    rendered.unmount();
  }
});
