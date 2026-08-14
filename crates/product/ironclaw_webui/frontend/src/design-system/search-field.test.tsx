import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { test } from "vitest";

import { SearchField } from "./search-field";

function childByType(node, type) {
  const children = Array.isArray(node.props.children)
    ? node.props.children
    : [node.props.children];
  return children.find((child) => child?.type === type);
}

test("SearchField is controlled and exposes localized accessible text", () => {
  const changes = [];
  let clears = 0;
  const rendered = SearchField({
    value: "calendar",
    onChange: (value) => changes.push(value),
    onClear: () => {
      clears += 1;
    },
    placeholder: "Search extensions…",
    "aria-label": "Search extensions",
    clearLabel: "Clear search",
  });

  const input = childByType(rendered, "input");
  const clearButton = childByType(rendered, "button");
  input.props.onChange({ currentTarget: { value: "github" } });
  clearButton.props.onClick();

  assert.deepEqual(changes, ["github"]);
  assert.equal(clears, 1);
  assert.equal(input.props.value, "calendar");
  assert.equal(input.props.placeholder, "Search extensions…");
  assert.equal(input.props["aria-label"], "Search extensions");
  assert.match(input.props.className, /appearance-none/);
  assert.match(input.props.className, /webkit-search-cancel-button/);
  assert.equal(clearButton.props["aria-label"], "Clear search");
});

test("SearchField consistently disables typing and clearing", () => {
  const html = renderToStaticMarkup(
    <SearchField
      value="calendar"
      onChange={() => {}}
      onClear={() => {}}
      placeholder="Search extensions…"
      aria-label="Search extensions"
      clearLabel="Clear search"
      disabled
    />,
  );

  assert.match(html, /<input[^>]*disabled=""/);
  assert.match(html, /<button[^>]*disabled=""/);
});

test("SearchField omits the optional clear action", () => {
  const html = renderToStaticMarkup(
    <SearchField
      value="calendar"
      onChange={() => {}}
      placeholder="Search extensions…"
      aria-label="Search extensions"
    />,
  );

  assert.doesNotMatch(html, /<button/);
});
