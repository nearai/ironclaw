// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { VerticalTabs, VerticalTabsMobile, type VerticalTabItem } from "./vertical-tabs";

const ITEMS: VerticalTabItem[] = [
  { id: "tools", label: "Tools", icon: "tool", count: 12 },
  { id: "skills", label: "Skills", icon: "file" },
];

test("VerticalTabs marks the active item and reports selection", () => {
  const container = document.createElement("div");
  document.body.append(container);
  const selected: string[] = [];

  try {
    const root = createRoot(container);
    act(() =>
      root.render(
        <VerticalTabs
          items={ITEMS}
          activeId="tools"
          onSelect={(id) => selected.push(id)}
          label="Settings sections"
        />
      )
    );

    const nav = container.querySelector("nav");
    assert.ok(nav);
    assert.equal(nav.getAttribute("aria-label"), "Settings sections");

    const buttons = Array.from(container.querySelectorAll("button"));
    assert.equal(buttons.length, 2);
    assert.equal(buttons[0].getAttribute("aria-current"), "true");
    assert.equal(buttons[1].getAttribute("aria-current"), null);
    assert.match(buttons[0].textContent ?? "", /12/);

    act(() => buttons[1].click());
    assert.deepEqual(selected, ["skills"]);
  } finally {
    container.remove();
  }
});

test("VerticalTabsMobile summarises the active item in the disclosure", () => {
  const container = document.createElement("div");
  document.body.append(container);

  try {
    const root = createRoot(container);
    act(() =>
      root.render(
        <VerticalTabsMobile
          items={ITEMS}
          activeId="skills"
          onSelect={() => {}}
          label="Settings sections"
        />
      )
    );

    const summary = container.querySelector("summary");
    assert.ok(summary);
    assert.match(summary.textContent ?? "", /Skills/);
    // Both items render in the expandable list below the summary.
    assert.equal(container.querySelectorAll("button").length, 2);
  } finally {
    container.remove();
  }
});
