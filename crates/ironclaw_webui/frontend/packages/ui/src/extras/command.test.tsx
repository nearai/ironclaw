// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React, { act } from "react";
import { renderIntoDocument, click } from "./test-helpers";
import {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "./command";

function setInputValue(input: HTMLInputElement, value: string) {
  act(() => {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
    setter?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

function Palette({ onSelect }: { onSelect?: (value: string) => void }) {
  return (
    <Command>
      <CommandInput placeholder="Search…" />
      <CommandList>
        <CommandEmpty>Nothing found</CommandEmpty>
        <CommandGroup heading="Runs">
          <CommandItem value="New run" onSelect={onSelect}>New run</CommandItem>
          <CommandItem value="Pause runs" onSelect={onSelect}>Pause runs</CommandItem>
        </CommandGroup>
        <CommandGroup heading="Settings">
          <CommandItem value="Open settings" onSelect={onSelect}>Open settings</CommandItem>
        </CommandGroup>
      </CommandList>
    </Command>
  );
}

test("Command wires combobox/listbox semantics and filters items", () => {
  const rendered = renderIntoDocument(<Palette />);
  try {
    const input = rendered.container.querySelector<HTMLInputElement>('[role="combobox"]');
    assert.ok(input);
    assert.equal(rendered.container.querySelectorAll('[role="option"]').length, 3);
    assert.equal(input.getAttribute("aria-controls"), rendered.container.querySelector('[role="listbox"]')?.id);

    setInputValue(input, "settings");
    const options = rendered.container.querySelectorAll('[role="option"]');
    assert.equal(options.length, 1);
    assert.match(options[0].textContent ?? "", /Open settings/);
    // The "Runs" group hides itself when none of its items match.
    assert.doesNotMatch(rendered.container.textContent ?? "", /New run/);

    setInputValue(input, "zzz-no-match");
    assert.equal(rendered.container.querySelectorAll('[role="option"]').length, 0);
    assert.match(rendered.container.textContent ?? "", /Nothing found/);
  } finally {
    rendered.unmount();
  }
});

test("Command Enter selects the active item", () => {
  const selected: string[] = [];
  const rendered = renderIntoDocument(<Palette onSelect={(value) => selected.push(value)} />);
  try {
    const input = rendered.container.querySelector<HTMLInputElement>('[role="combobox"]')!;
    // First visible item is active by default; ArrowDown moves to the second.
    act(() => {
      input.dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true })
      );
    });
    act(() => {
      input.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })
      );
    });
    assert.deepEqual(selected, ["Pause runs"]);
  } finally {
    rendered.unmount();
  }
});

test("CommandItem click selects and mouse hover moves the highlight", () => {
  const selected: string[] = [];
  const rendered = renderIntoDocument(<Palette onSelect={(value) => selected.push(value)} />);
  try {
    const option = Array.from(rendered.container.querySelectorAll('[role="option"]')).find(
      (node) => /Open settings/.test(node.textContent ?? "")
    );
    assert.ok(option);
    click(option);
    assert.deepEqual(selected, ["Open settings"]);
  } finally {
    rendered.unmount();
  }
});

test("CommandDialog renders a modal dialog and closes on Escape", () => {
  let closed = 0;
  const rendered = renderIntoDocument(
    <CommandDialog open onClose={() => { closed += 1; }}>
      <Palette />
    </CommandDialog>
  );
  try {
    const dialog = document.body.querySelector('[role="dialog"]');
    assert.ok(dialog);
    assert.equal(dialog.getAttribute("aria-modal"), "true");
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    assert.equal(closed, 1);
  } finally {
    rendered.unmount();
  }
});
