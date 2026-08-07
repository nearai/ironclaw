// @vitest-environment happy-dom
import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { test, vi } from "vitest";

import { useFilePicker } from "./useFilePicker";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

function PickerHarness({ disabled = false, onSelect }) {
  const [openFilePicker, inputProps] = useFilePicker({
    accept: ".json,application/json",
    multiple: true,
    disabled,
    onSelect,
  });

  return (
    <>
      <button type="button" onClick={openFilePicker}>Choose</button>
      <input data-testid="file-input" {...inputProps} />
    </>
  );
}

function renderPicker(props) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<PickerHarness {...props} />));
  return {
    button: container.querySelector("button"),
    input: container.querySelector("input"),
    cleanup: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

function selectFiles(input, files) {
  Object.defineProperty(input, "files", {
    configurable: true,
    value: files,
  });
  Object.defineProperty(input, "value", {
    configurable: true,
    writable: true,
    value: files.length > 0 ? `C:\\fakepath\\${files[0].name}` : "",
  });
  act(() => input.dispatchEvent(new Event("change", { bubbles: true })));
}

test("useFilePicker opens the native input and forwards its configuration", () => {
  const click = vi.spyOn(HTMLInputElement.prototype, "click");
  const picker = renderPicker({ onSelect: () => {} });

  try {
    assert.ok(picker.button);
    assert.ok(picker.input);
    act(() => picker.button.click());

    assert.equal(click.mock.calls.length, 1);
    assert.equal(picker.input.accept, ".json,application/json");
    assert.equal(picker.input.multiple, true);
    assert.equal(picker.input.hidden, true);
    assert.equal(picker.input.disabled, false);
  } finally {
    picker.cleanup();
    click.mockRestore();
  }
});

test("useFilePicker resets after selection, supports reselection, and ignores cancel", () => {
  const onSelect = vi.fn();
  const picker = renderPicker({ onSelect });
  const file = new File(["{}"], "settings.json", { type: "application/json" });

  try {
    selectFiles(picker.input, [file]);
    assert.equal(picker.input.value, "");
    selectFiles(picker.input, [file]);
    selectFiles(picker.input, []);

    assert.equal(onSelect.mock.calls.length, 2);
    assert.deepEqual(onSelect.mock.calls[0][0], [file]);
    assert.deepEqual(onSelect.mock.calls[1][0], [file]);
  } finally {
    picker.cleanup();
  }
});

test("useFilePicker prevents disabled activation and selection", () => {
  const click = vi.spyOn(HTMLInputElement.prototype, "click");
  const onSelect = vi.fn();
  const picker = renderPicker({ disabled: true, onSelect });

  try {
    assert.ok(picker.button);
    assert.ok(picker.input);
    act(() => picker.button.click());
    selectFiles(picker.input, [new File(["{}"], "settings.json")]);

    assert.equal(click.mock.calls.length, 0);
    assert.equal(onSelect.mock.calls.length, 0);
    assert.equal(picker.input.disabled, true);
    assert.equal(picker.input.value, "");
  } finally {
    picker.cleanup();
    click.mockRestore();
  }
});
