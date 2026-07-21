import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { test } from "vitest";
import { SelectMenu } from "./select-menu";

const OPTIONS = [
  { value: "next-run", label: "Next run" },
  { value: "name", label: "Name", disabled: true },
  { value: "recent", label: "Recently created", tone: "positive" as const },
];

test("SelectMenu renders a combobox trigger with the selected label", () => {
  const html = renderToStaticMarkup(
    <SelectMenu
      ariaLabel="Sort automations"
      prefix="Sort"
      value="next-run"
      options={OPTIONS}
      onChange={() => {}}
    />,
  );

  assert.match(html, /role="combobox"/);
  assert.match(html, /aria-label="Sort automations"/);
  assert.match(html, /Sort/);
  assert.match(html, /Next run/);
});

test("SelectMenu disables the trigger when every option is disabled", () => {
  const html = renderToStaticMarkup(
    <SelectMenu
      ariaLabel="Empty"
      value="a"
      options={[{ value: "a", label: "A", disabled: true }]}
      onChange={() => {}}
    />,
  );

  assert.match(html, /disabled/);
});

test("SelectMenu passes through data attributes on the root", () => {
  const html = renderToStaticMarkup(
    <SelectMenu
      data-testid="sort-menu"
      ariaLabel="Sort"
      value="next-run"
      options={OPTIONS}
      onChange={() => {}}
    />,
  );

  assert.match(html, /data-testid="sort-menu"/);
});
