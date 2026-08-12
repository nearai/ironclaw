// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../../test-support/vm-module-harness";

function visit(node, fn) {
  if (Array.isArray(node)) {
    for (const item of node) visit(item, fn);
    return;
  }
  if (!node || typeof node !== "object") return;
  fn(node);
  if (Array.isArray(node.values)) {
    for (const value of node.values) visit(value, fn);
  }
}

function findComponentNode(root, component) {
  let found = null;
  visit(root, (node) => {
    if (!found && Array.isArray(node.values) && node.values.includes(component)) {
      found = node;
    }
  });
  return found;
}

function componentProps(node, component) {
  const props = {};
  const start = node.values.indexOf(component);
  for (let index = start + 1; index < node.values.length; index += 1) {
    const name = node.strings[index]?.match(/([A-Za-z][A-Za-z0-9-]*)=\s*$/)?.[1];
    if (name) props[name] = node.values[index];
  }
  return props;
}

function component(name) {
  return function TestComponent() {
    return name;
  };
}

function renderSettingsField() {
  const Switch = component("Switch");
  const context = {
    Card: component("Card"),
    React: {
      useCallback: (callback) => callback,
      useEffect: () => {},
      useState: (initialValue) => [initialValue, () => {}],
    },
    Switch,
    useT: () => (key) => key,
  };
  const exports = runVmModuleForTest(
    "./settings-field.tsx",
    ["SettingsField"],
    context,
    import.meta.url
  );
  return { exports, Switch };
}

test("SettingsField uses the compact shared Switch and preserves string persistence", () => {
  const { exports, Switch } = renderSettingsField();
  const saved = [];
  const rendered = exports.SettingsField({
    field: {
      key: "agent.use_planning",
      label: "Enable planning",
      type: "boolean",
    },
    value: "true",
    onSave: (key, value) => saved.push({ key, value }),
    isSaved: false,
  });
  const switchNode = findComponentNode(rendered, Switch);

  assert.ok(switchNode, "expected boolean settings field to render the shared Switch");
  const props = componentProps(switchNode, Switch);
  assert.equal(props.checked, true);
  assert.equal(props.size, "sm");
  assert.equal(props["aria-label"], "Enable planning");

  props.onChange(false);
  assert.deepEqual(saved, [{ key: "agent.use_planning", value: "false" }]);
});
