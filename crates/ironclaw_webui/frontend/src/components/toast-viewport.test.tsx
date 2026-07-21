import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../test-support/vm-module-harness";

function visit(node, fn) {
  if (Array.isArray(node)) {
    for (const child of node) visit(child, fn);
    return;
  }
  if (!node || typeof node !== "object") return;
  fn(node);
  visit(node.children, fn);
}

function nodeBy(root, predicate, description) {
  let match = null;
  visit(root, (node) => {
    if (!match && predicate(node)) match = node;
  });
  assert.ok(match, `expected ${description}`);
  return match;
}

test("ToastViewport wires the lifecycle provider with accessible dismissible content", () => {
  const cleanups = [];
  const dismissals = [];
  let removed = false;
  function Toaster() {}
  const hotToast = {
    dismiss: (id) => dismissals.push(id),
    remove: () => {
      removed = true;
    },
  };
  const context = {
    React: {
      useEffect: (effect) => cleanups.push(effect()),
    },
    hotToast,
    Toaster,
    resolveValue: (message) => message,
    Icon: ({ name }) => ({ type: "icon", props: { name }, children: [] }),
    useT: () => (key) => (key === "common.dismiss" ? "Dismiss" : key),
  };
  const { ToastViewport } = runVmModuleForTest(
    "./toast-viewport.tsx",
    ["ToastViewport"],
    context,
    import.meta.url
  );

  const rendered = ToastViewport();
  assert.equal(rendered.type, Toaster);
  assert.equal(rendered.props.position, "bottom-right");
  assert.equal(rendered.props.containerStyle.zIndex, 10000);

  const renderToast = rendered.children[0];
  const alert = renderToast({
    id: "toast-1",
    type: "error",
    visible: true,
    message: "Could not save",
    ariaProps: { role: "alert", "aria-live": "assertive" },
  });
  assert.equal(alert.props.role, "alert");
  assert.equal(alert.props["aria-live"], "assertive");
  assert.match(alert.props.className, /opacity-100/);

  const dismiss = nodeBy(
    alert,
    (node) => node.type === "button" && node.props?.["aria-label"] === "Dismiss",
    "dismiss button"
  );
  dismiss.props.onClick();
  assert.deepEqual(dismissals, ["toast-1"]);

  cleanups.forEach((cleanup) => cleanup?.());
  assert.equal(removed, true, "unmount removes stale global toast entries");
});
