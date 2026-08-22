// @vitest-environment happy-dom
// @ts-nocheck
import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { test, vi } from "vitest";

import { useCopyToClipboard } from "./useCopyToClipboard";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

function CopyHarness({ text, resetMs, onResult }) {
  const { copied, copy } = useCopyToClipboard(resetMs);

  return (
    <>
      <button
        type="button"
        onClick={async () => {
          onResult(await copy(text));
        }}
      >
        Copy
      </button>
      <span data-testid="state">{copied ? "copied" : "idle"}</span>
    </>
  );
}

function renderHarness({ text, resetMs, writeText }) {
  const results = [];
  vi.stubGlobal("navigator", { clipboard: { writeText } });

  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => {
    root.render(
      <CopyHarness text={text} resetMs={resetMs} onResult={(ok) => results.push(ok)} />,
    );
  });

  return {
    results,
    state: () => container.querySelector("[data-testid='state']").textContent,
    click: async () => {
      await act(async () => {
        container.querySelector("button").click();
      });
    },
    cleanup: () => {
      act(() => {
        root.unmount();
      });
      container.remove();
      vi.unstubAllGlobals();
      vi.useRealTimers();
    },
  };
}

test("a successful copy writes the text and reports success", async () => {
  const written = [];
  const harness = renderHarness({
    text: "https://agent.example.com/api/ironhub/register",
    writeText: async (value) => {
      written.push(value);
    },
  });

  await harness.click();

  assert.deepEqual(written, ["https://agent.example.com/api/ironhub/register"]);
  assert.deepEqual(harness.results, [true]);
  assert.equal(harness.state(), "copied");
  harness.cleanup();
});

test("a rejected clipboard write reports failure and never shows a copied state", async () => {
  const harness = renderHarness({
    text: "https://agent.example.com/api/ironhub/register",
    writeText: async () => {
      throw new Error("denied");
    },
  });

  await harness.click();

  assert.deepEqual(harness.results, [false]);
  assert.equal(
    harness.state(),
    "idle",
    "a blocked clipboard must not claim the value was copied",
  );
  harness.cleanup();
});

test("empty text is a no-op rather than a copy of nothing", async () => {
  const written = [];
  const harness = renderHarness({
    text: "",
    writeText: async (value) => {
      written.push(value);
    },
  });

  await harness.click();

  assert.deepEqual(written, []);
  assert.deepEqual(harness.results, [false]);
  assert.equal(harness.state(), "idle");
  harness.cleanup();
});

test("the copied state resets once the reset window elapses", async () => {
  vi.useFakeTimers();
  try {
    const harness = renderHarness({
      text: "https://agent.example.com/api/ironhub/register",
      resetMs: 50,
      writeText: async () => {},
    });

    await harness.click();
    assert.equal(harness.state(), "copied");

    await act(async () => {
      vi.advanceTimersByTime(50);
    });

    assert.equal(harness.state(), "idle");
  } finally {
    vi.useRealTimers();
  }
});
