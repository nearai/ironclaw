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
  const Input = component("Input");
  const SelectMenu = component("SelectMenu");
  const Switch = component("Switch");
  const context = {
    Card: component("Card"),
    Input,
    React: {
      useCallback: (callback) => callback,
      useEffect: () => {},
      useState: (initialValue) => [initialValue, () => {}],
    },
    SelectMenu,
    Switch,
    useT: () => (key) => key,
  };
  const exports = runVmModuleForTest(
    "./settings-field.tsx",
    ["SettingsField"],
    context,
    import.meta.url
  );
  return { exports, Input, SelectMenu, Switch };
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

test("SettingsField uses the shared Input and preserves numeric constraints and commit behavior", () => {
  const { exports, Input } = renderSettingsField();
  const saved = [];
  const rendered = exports.SettingsField({
    field: {
      key: "search.weight",
      label: "Search weight",
      type: "float",
      min: 0,
      max: 1,
      step: 0.1,
    },
    value: 0.5,
    onSave: (key, value) => saved.push({ key, value }),
    isSaved: false,
  });
  const inputNode = findComponentNode(rendered, Input);

  assert.ok(inputNode, "expected numeric settings field to render the shared Input");
  const props = componentProps(inputNode, Input);
  assert.equal(props.type, "number");
  assert.equal(props.size, "sm");
  assert.equal(props.className, "text-right font-mono");
  assert.equal(props.step, "0.1");
  assert.equal(props.min, "0");
  assert.equal(props.max, "1");
  assert.equal(props.placeholder, "tools.default");
  assert.equal(props["aria-label"], "Search weight");

  props.onBlur({ currentTarget: { value: "0.75" } });
  props.onKeyDown({ key: "Escape", currentTarget: { value: "0.9" } });
  props.onKeyDown({ key: "Enter", currentTarget: { value: "0.9" } });
  assert.deepEqual(saved, [
    { key: "search.weight", value: 0.75 },
    { key: "search.weight", value: 0.9 },
  ]);
});

test("SettingsField uses the shared SelectMenu and preserves the default option", () => {
  const { exports, SelectMenu } = renderSettingsField();
  const saved = [];
  const rendered = exports.SettingsField({
    field: {
      key: "search.fusion_strategy",
      label: "Fusion strategy",
      type: "select",
      options: ["rrf", "weighted"],
    },
    value: "rrf",
    onSave: (key, value) => saved.push({ key, value }),
    isSaved: false,
  });
  const selectNode = findComponentNode(rendered, SelectMenu);

  assert.ok(selectNode, "expected select settings field to render the shared SelectMenu");
  const props = componentProps(selectNode, SelectMenu);
  assert.equal(props.ariaLabel, "Fusion strategy");
  assert.equal(props.className, "!min-w-0 w-36");
  assert.deepEqual(JSON.parse(JSON.stringify(props.options)), [
    { label: "tools.default", value: "" },
    { label: "rrf", value: "rrf" },
    { label: "weighted", value: "weighted" },
  ]);

  props.onChange("weighted");
  props.onChange("");
  assert.deepEqual(saved, [
    { key: "search.fusion_strategy", value: "weighted" },
    { key: "search.fusion_strategy", value: null },
  ]);
});
