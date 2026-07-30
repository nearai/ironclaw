// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { DetailList, DetailRow } from "./detail-list";

test("DetailList renders semantic dt/dd rows with separators after the first", () => {
  const container = document.createElement("div");
  document.body.append(container);

  try {
    const root = createRoot(container);
    act(() =>
      root.render(
        <DetailList>
          <DetailRow term="ID">usr-1</DetailRow>
          <DetailRow term="Email">a@b.c</DetailRow>
          <DetailRow layout="stacked" term="Created">Jul 29</DetailRow>
        </DetailList>
      )
    );

    const list = container.querySelector("dl");
    assert.ok(list);
    const terms = Array.from(container.querySelectorAll("dt")).map((el) => el.textContent);
    const values = Array.from(container.querySelectorAll("dd")).map((el) => el.textContent);
    assert.deepEqual(terms, ["ID", "Email", "Created"]);
    assert.deepEqual(values, ["usr-1", "a@b.c", "Jul 29"]);

    const rows = Array.from(list.children);
    assert.equal(rows.length, 3);
    // Every row carries the hairline class; CSS first: strips it on row one.
    for (const row of rows) {
      assert.match(row.className, /border-t/);
      assert.match(row.className, /first:border-0/);
    }
    // Stacked layout swaps the side-by-side row for an eyebrow label.
    assert.match(rows[2].querySelector("dt")?.className ?? "", /font-mono/);
  } finally {
    container.remove();
  }
});
