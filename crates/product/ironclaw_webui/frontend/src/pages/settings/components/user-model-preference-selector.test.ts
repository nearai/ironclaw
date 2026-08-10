// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../../test-support/vm-module-harness";

function html(strings, ...values) {
  return { strings: Array.from(strings), values };
}

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

test("model preference selector contains long model names inside its card", () => {
  const SelectMenu = "SelectMenu";
  const exports = runVmModuleForTest(
    "./user-model-preference-selector.tsx",
    ["UserModelPreferenceSelector"],
    {
      Card: "Card",
      SelectMenu,
      html,
      useT: () => (key, params = {}) =>
        key === "llm.followWorkspaceDefault"
          ? `Workspace default (${params.model})`
          : key,
      useUserModelPreference: () => ({
        catalog: {
          selection_enabled: true,
          workspace_default: "deepseek-ai/DeepSeek-V4-Flash-with-a-very-long-name",
          models: ["deepseek-ai/DeepSeek-V4-Flash-with-a-very-long-name"],
        },
        model: null,
        isLoading: false,
        isSaving: false,
        error: null,
        setModel: () => {},
      }),
    },
    import.meta.url
  );

  const rendered = exports.UserModelPreferenceSelector();
  const selectNode = findComponentNode(rendered, SelectMenu);
  assert.ok(selectNode, "expected model selector");

  const props = componentProps(selectNode, SelectMenu);
  assert.match(props.className, /\bw-full\b/);
  assert.match(props.className, /\bmax-w-full\b/);
  assert.match(props.className, /\bmin-w-0\b/);
  assert.match(props.buttonClassName, /\boverflow-hidden\b/);
  assert.match(props.menuClassName, /\bw-full\b/);
});
