// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";
import vm from "node:vm";
import { componentSourceForTest, findComponent } from "../../../lib/vm-component-harness";

// Same vm-harness convention: the surface's only free variables are the feature
// hook, `useT`, and the child card, so we stub the hook to force the gate on/off
// and assert what the surface renders. This is the flag-gating regression: the
// surface must be inert (render null) for real users until `oobe_suggestions`.
function surfaceSourceForTest() {
  return componentSourceForTest(
    new URL("./suggested-task-surface.tsx", import.meta.url),
    "SuggestedTaskSurface",
  );
}

function renderSurface({ enabled }) {
  const components = { SuggestedTaskCard() {} };
  const context = {
    ...components,
    globalThis: {},
    useT: () => (key) => key,
    useOobeSuggestionsEnabled: () => enabled,
  };
  vm.runInNewContext(surfaceSourceForTest(), context);
  const tree = context.globalThis.__testExports.SuggestedTaskSurface({});
  return { tree, components };
}

test("surface renders nothing when the oobe_suggestions flag is off", () => {
  const { tree } = renderSurface({ enabled: false });
  assert.equal(tree, null, "the landing must be unchanged for real users when the flag is off");
});

test("surface renders the demo cards when the flag is on", () => {
  const { tree, components } = renderSurface({ enabled: true });
  assert.notEqual(tree, null);
  assert.ok(
    findComponent(tree, components.SuggestedTaskCard),
    "the flag-on surface should mount SuggestedTaskCard(s)",
  );
});
