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

function collectTemplateText(root) {
  const text = [];
  visit(root, (node) => {
    if (Array.isArray(node.strings)) text.push(...node.strings);
  });
  return text.join("");
}

function collectRenderedText(root) {
  const text = [];
  visit(root, (node) => {
    if (!Array.isArray(node.values)) return;
    for (const value of node.values) {
      if (typeof value === "string") text.push(value);
    }
  });
  return text.join(" ");
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

  const markup = collectTemplateText(rendered);
  assert.match(markup, /flex flex-col gap-4 xl:flex-row/);
  assert.doesNotMatch(markup, /\bsm:flex-row\b/);
  assert.match(
    markup,
    /w-full min-w-0 xl:ml-auto xl:w-72 xl:max-w-full xl:flex-none/
  );

  const props = componentProps(selectNode, SelectMenu);
  assert.match(props.className, /\bw-full\b/);
  assert.match(props.className, /\bmax-w-full\b/);
  assert.match(props.className, /\bmin-w-0\b/);
  assert.match(props.buttonClassName, /\boverflow-hidden\b/);
  assert.match(props.menuClassName, /\bw-full\b/);
});

test("stale model preference can be reset when selection policy is unavailable", () => {
  const SelectMenu = "SelectMenu";
  const staleModel = "provider-a/retired-model";
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
          : key === "llm.unavailableModel"
            ? `Unavailable (${params.model})`
            : key,
      useUserModelPreference: () => ({
        catalog: {
          selection_enabled: false,
          workspace_default: null,
          models: [],
        },
        model: staleModel,
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

  assert.equal(
    props.disabled,
    false,
    "the reset option must remain usable when a stale preference exists"
  );
  assert.deepEqual(JSON.parse(JSON.stringify(props.options)), [
    {
      value: "",
      label: "Workspace default (inference.none)",
    },
    {
      value: staleModel,
      label: `Unavailable (${staleModel})`,
      disabled: true,
      tone: "warning",
    },
  ]);
});

test("model preference selector blocks writes when the preference read fails", () => {
  const SelectMenu = "SelectMenu";
  const exports = runVmModuleForTest(
    "./user-model-preference-selector.tsx",
    ["UserModelPreferenceSelector"],
    {
      Card: "Card",
      SelectMenu,
      html,
      useT: () => (key) => key,
      useUserModelPreference: () => ({
        catalog: {
          selection_enabled: true,
          workspace_default: "model-a",
          models: ["model-a"],
        },
        model: null,
        isLoading: false,
        isSaving: false,
        preferenceReadFailed: true,
        error: new Error("preference read failed"),
        setModel: () => {},
      }),
    },
    import.meta.url
  );

  const rendered = exports.UserModelPreferenceSelector();
  const selectNode = findComponentNode(rendered, SelectMenu);
  assert.ok(selectNode, "expected model selector");
  const props = componentProps(selectNode, SelectMenu);

  assert.equal(
    props.disabled,
    true,
    "a failed preference read must prevent an uninformed overwrite"
  );
});

for (const {
  name,
  catalogReadFailed,
  preferenceReadFailed,
  selectionEnabled,
  expectedStatus,
} of [
  {
    name: "catalog",
    catalogReadFailed: true,
    preferenceReadFailed: false,
    selectionEnabled: false,
    expectedStatus: "llm.catalogLoadFailed",
  },
  {
    name: "preference",
    catalogReadFailed: false,
    preferenceReadFailed: true,
    selectionEnabled: true,
    expectedStatus: "llm.preferenceLoadFailed",
  },
]) {
  test(`${name} read failures render a load failure before policy status`, () => {
    const exports = runVmModuleForTest(
      "./user-model-preference-selector.tsx",
      ["UserModelPreferenceSelector"],
      {
        ApiError: class ApiError extends Error {},
        Card: "Card",
        SelectMenu: "SelectMenu",
        html,
        useT: () => (key) => key,
        useUserModelPreference: () => ({
          catalog: {
            selection_enabled: selectionEnabled,
            workspace_default: "model-a",
            models: ["model-a"],
          },
          model: null,
          isLoading: false,
          isSaving: false,
          catalogReadFailed,
          preferenceReadFailed,
          saveError: null,
          error: new Error(`${name} read failed`),
          setModel: () => {},
        }),
      },
      import.meta.url
    );

    const text = collectRenderedText(exports.UserModelPreferenceSelector());
    assert.match(text, new RegExp(expectedStatus.replace(".", "\\.")));
    assert.doesNotMatch(text, /llm\.selectionUnavailable/);
    assert.doesNotMatch(text, /error\.saveFailed/);
  });
}
