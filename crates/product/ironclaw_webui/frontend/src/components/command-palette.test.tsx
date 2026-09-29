// @vitest-environment happy-dom

import assert from "node:assert/strict";
import React, { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { MemoryRouter } from "react-router";
import { test } from "vitest";

import { CommandPalette } from "./command-palette";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

test("CommandPalette restores focus to its opener after Escape", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);

  function Harness() {
    const [open, setOpen] = useState(false);
    return (
      <MemoryRouter>
        <button onClick={() => setOpen(true)}>Open palette</button>
        <CommandPalette
          open={open}
          onClose={() => setOpen(false)}
          threadsState={{ threads: [] }}
          onNewChat={() => {}}
          onToggleTheme={() => {}}
        />
      </MemoryRouter>
    );
  }

  try {
    act(() => root.render(<Harness />));
    const opener = container.querySelector("button");
    assert.ok(opener);
    opener.focus();
    const focused = new Promise<void>((resolve) => {
      container.addEventListener("focusin", function onFocus(event) {
        if (event.target instanceof HTMLInputElement) {
          container.removeEventListener("focusin", onFocus);
          resolve();
        }
      });
    });
    act(() => opener.click());
    await focused;
    const input = container.querySelector("input");
    assert.ok(input);
    assert.ok(document.activeElement === input, "opening focuses the palette input");

    act(() => {
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });

    assert.equal(container.querySelector('[role="dialog"]'), null);
    assert.ok(document.activeElement === opener, "Escape returns focus to the opener");
  } finally {
    act(() => root.unmount());
    container.remove();
  }
});
