// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { SearchInput } from "./search-input";

test("SearchInput renders an accessible label and clears via the trailing button", () => {
  const container = document.createElement("div");
  document.body.append(container);
  let cleared = 0;

  try {
    const root = createRoot(container);
    act(() =>
      root.render(
        <SearchInput
          label="Search jobs"
          value="nightly"
          onChange={() => {}}
          onClear={() => {
            cleared += 1;
          }}
          clearLabel="Clear search"
        />
      )
    );

    const input = container.querySelector("input");
    assert.ok(input);
    assert.equal(input.type, "search");
    assert.match(container.querySelector("label")?.textContent ?? "", /Search jobs/);

    const clear = container.querySelector('button[aria-label="Clear search"]');
    assert.ok(clear, "clear button renders while a value is set");
    act(() => (clear as HTMLButtonElement).click());
    assert.equal(cleared, 1);
  } finally {
    container.remove();
  }
});

test("SearchInput hides the clear button when empty", () => {
  const container = document.createElement("div");
  document.body.append(container);

  try {
    const root = createRoot(container);
    act(() =>
      root.render(
        <SearchInput label="Search" value="" onChange={() => {}} onClear={() => {}} />
      )
    );
    assert.equal(container.querySelector('button'), null);
  } finally {
    container.remove();
  }
});
