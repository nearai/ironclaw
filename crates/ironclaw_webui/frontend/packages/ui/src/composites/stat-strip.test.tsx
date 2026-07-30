// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { StatStrip, StatTile } from "./stat-strip";

test("StatTile renders a static div without onSelect and a pressed button with it", () => {
  const container = document.createElement("div");
  document.body.append(container);
  const selections: string[] = [];

  try {
    const root = createRoot(container);
    act(() =>
      root.render(
        <StatStrip columns={2}>
          <StatTile label="Scheduled" value={6} badgeLabel="idle" />
          <StatTile
            label="Failures"
            value={1}
            tone="danger"
            badgeLabel="failing"
            onSelect={() => selections.push("failures")}
            isActive
            selectTitle="Filter failures"
          />
        </StatStrip>
      )
    );

    const buttons = Array.from(container.querySelectorAll("button"));
    assert.equal(buttons.length, 1, "only the interactive tile is a button");
    assert.equal(buttons[0].getAttribute("aria-pressed"), "true");
    assert.equal(buttons[0].getAttribute("title"), "Filter failures");

    act(() => buttons[0].click());
    assert.deepEqual(selections, ["failures"]);

    assert.match(container.textContent ?? "", /Scheduled.*6/s);
    assert.match(container.textContent ?? "", /Failures.*1/s);
  } finally {
    container.remove();
  }
});
