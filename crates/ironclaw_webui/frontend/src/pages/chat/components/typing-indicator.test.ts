// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";
import vm from "node:vm";
import {
  componentProps,
  componentSourceForTest,
  findComponent,
} from "../../../lib/vm-component-harness";

function typingIndicatorSourceForTest() {
  return componentSourceForTest(
    new URL("./typing-indicator.tsx", import.meta.url),
    "TypingIndicator",
  );
}

test("TypingIndicator keeps the brief action label beside the working indicator", () => {
  const components = {
    NearProcessIndicator() {},
  };
  const context = {
    ...components,
    globalThis: {},
  };

  vm.runInNewContext(typingIndicatorSourceForTest(), context);
  const tree = context.globalThis.__testExports.TypingIndicator();
  const indicator = findComponent(tree, components.NearProcessIndicator);

  assert.deepEqual(
    componentProps(indicator, components.NearProcessIndicator),
    {
      state: "working",
      label: "Working…",
    },
  );
});

test("TypingIndicator keeps the static mark with elapsed time after completion", () => {
  const components = {
    NearProcessIndicator() {},
  };
  const context = {
    ...components,
    globalThis: {},
  };

  vm.runInNewContext(typingIndicatorSourceForTest(), context);
  const tree = context.globalThis.__testExports.TypingIndicator({
    state: "done",
    durationSeconds: 12,
  });
  const indicator = findComponent(tree, components.NearProcessIndicator);

  assert.deepEqual(
    componentProps(indicator, components.NearProcessIndicator),
    {
      state: "done",
      label: "Worked for 12s",
    },
  );
});

test.each([
  [60, "Worked for 00:01:00"],
  [3_661, "Worked for 01:01:01"],
])(
  "TypingIndicator formats completed runs of %i seconds as HH:MM:SS",
  (durationSeconds, label) => {
    const components = {
      NearProcessIndicator() {},
    };
    const context = {
      ...components,
      globalThis: {},
    };

    vm.runInNewContext(typingIndicatorSourceForTest(), context);
    const tree = context.globalThis.__testExports.TypingIndicator({
      state: "done",
      durationSeconds,
    });
    const indicator = findComponent(tree, components.NearProcessIndicator);

    assert.deepEqual(
      componentProps(indicator, components.NearProcessIndicator),
      {
        state: "done",
        label,
      },
    );
  },
);
