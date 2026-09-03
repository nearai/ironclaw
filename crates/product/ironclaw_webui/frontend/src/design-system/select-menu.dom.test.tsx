// @vitest-environment happy-dom

import assert from "node:assert/strict";
import React, { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { test } from "vitest";

import { SelectMenu } from "./select-menu";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

test("SelectMenu exposes a valid select-only combobox relationship in the DOM", () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);

  function Harness() {
    const [value, setValue] = useState("sandbox");
    return (
      <SelectMenu
        value={value}
        onChange={setValue}
        aria-label="Execution policy"
        options={[
          { value: "sandbox", label: "Sandbox policy" },
          { value: "fusion", label: "Fusion strategy" },
          { value: "tunnel", label: "Tunnel provider" },
        ]}
      />
    );
  }

  try {
    act(() => root.render(<Harness />));

    const combobox = container.querySelector<HTMLButtonElement>(
      'button[role="combobox"]',
    );
    assert.ok(combobox, "expected the trigger to expose the combobox role");
    assert.equal(combobox.getAttribute("aria-label"), "Execution policy");
    assert.match(combobox.textContent ?? "", /Sandbox policy/);
    const closedListboxId = combobox.getAttribute("aria-controls");
    assert.ok(closedListboxId, "expected the combobox to always reference its popup");
    const closedListbox = document.getElementById(closedListboxId);
    assert.equal(closedListbox?.getAttribute("role"), "listbox");
    assert.equal(closedListbox?.hidden, true);

    act(() => {
      combobox.focus();
      combobox.dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }),
      );
    });

    assert.equal(document.activeElement, combobox);
    assert.equal(combobox.getAttribute("aria-expanded"), "true");
    assert.equal(combobox.getAttribute("aria-controls"), closedListboxId);
    const openListbox = document.getElementById(closedListboxId);
    assert.equal(openListbox?.getAttribute("role"), "listbox");
    assert.equal(openListbox?.hidden, false);

    const activeOptionId = combobox.getAttribute("aria-activedescendant");
    assert.ok(activeOptionId, "expected the open combobox to expose its active option");
    const activeOption = document.getElementById(activeOptionId);
    assert.equal(activeOption?.getAttribute("role"), "option");
    assert.match(activeOption?.textContent ?? "", /Fusion strategy/);

    act(() => {
      combobox.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
      );
    });

    assert.equal(combobox.getAttribute("aria-expanded"), "false");
    assert.match(combobox.textContent ?? "", /Fusion strategy/);
  } finally {
    act(() => root.unmount());
    container.remove();
  }
});
