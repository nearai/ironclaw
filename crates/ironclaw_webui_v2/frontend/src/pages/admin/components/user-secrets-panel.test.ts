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

function collectScalars(root) {
  const scalars = [];
  visit(root, (node) => {
    if (!Array.isArray(node.values)) return;
    for (const value of node.values) {
      if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
        scalars.push(value);
      }
    }
  });
  return scalars;
}

function findByTestId(root, testId, handle) {
  let found = null;
  visit(root, (node) => {
    if (found || node.props?.["data-testid"] !== testId) return;
    if (handle !== undefined && node.props?.["data-secret-handle"] !== handle) return;
    found = node;
  });
  return found;
}

function createHarness(overrides = {}) {
  const state = [];
  let cursor = 0;
  const calls = { put: [], delete: [] };
  const React = {
    useState(initial) {
      const index = cursor;
      cursor += 1;
      if (!(index in state)) state[index] = typeof initial === "function" ? initial() : initial;
      return [
        state[index],
        (next) => {
          state[index] = typeof next === "function" ? next(state[index]) : next;
        },
      ];
    },
  };
  const translate = (key, params = {}) => {
    if (params.handle) return `${key}:${params.handle}`;
    if (params.message) return `${key}:${params.message}`;
    return key;
  };
  const exports = runVmModuleForTest(
    "./user-secrets-panel.tsx",
    ["UserSecretsPanelView"],
    {
      Button: function Button() {},
      Input: function Input() {},
      Panel: function Panel() {},
      React,
      useT: () => translate,
    },
    import.meta.url,
  );
  const props = {
    secrets: [],
    query: { isLoading: false, error: null },
    putSecret: async (handle, value) => calls.put.push({ handle, value }),
    deleteSecret: async (handle) => calls.delete.push(handle),
    isSaving: false,
    isDeleting: false,
    mutationError: null,
    ...overrides,
  };

  return {
    calls,
    render() {
      cursor = 0;
      return exports.UserSecretsPanelView(props);
    },
  };
}

test("secrets panel renders handles without exposing returned secret material", () => {
  const sensitive = "must-never-appear-in-the-panel";
  const harness = createHarness({
    secrets: [{ handle: "openai_api_key", value: sensitive, material: sensitive }],
  });

  const rendered = harness.render();
  const scalars = collectScalars(rendered);
  assert.ok(scalars.includes("openai_api_key"));
  assert.ok(!scalars.includes(sensitive));
  assert.ok(findByTestId(rendered, "admin-secret-row", "openai_api_key"));
});

test("secrets panel saves an exact write-only value and clears the form on success", async () => {
  const harness = createHarness();
  let rendered = harness.render();

  findByTestId(rendered, "admin-secret-handle").props.onChange({
    currentTarget: { value: "  openai_api_key  " },
  });
  rendered = harness.render();
  findByTestId(rendered, "admin-secret-value").props.onChange({
    currentTarget: { value: "  keep-secret-whitespace  " },
  });
  rendered = harness.render();
  const form = (() => {
    let found = null;
    visit(rendered, (node) => {
      if (!found && node.type === "form") found = node;
    });
    return found;
  })();
  await form.props.onSubmit({ preventDefault() {} });

  assert.deepEqual(harness.calls.put, [
    { handle: "openai_api_key", value: "  keep-secret-whitespace  " },
  ]);
  rendered = harness.render();
  assert.equal(findByTestId(rendered, "admin-secret-handle").props.value, "");
  assert.equal(findByTestId(rendered, "admin-secret-value").props.value, "");
  assert.ok(collectScalars(rendered).includes("admin.user.secrets.saved:openai_api_key"));
});

test("secrets panel requires confirmation before deleting a handle", async () => {
  const harness = createHarness({ secrets: [{ handle: "github_token" }] });
  let rendered = harness.render();

  findByTestId(rendered, "admin-secret-delete", "github_token").props.onClick();
  assert.deepEqual(harness.calls.delete, []);
  rendered = harness.render();
  assert.ok(findByTestId(rendered, "admin-secret-delete-dialog"));

  await findByTestId(rendered, "admin-secret-delete-confirm").props.onClick();
  assert.deepEqual(harness.calls.delete, ["github_token"]);
  rendered = harness.render();
  assert.equal(findByTestId(rendered, "admin-secret-delete-dialog"), null);
  assert.ok(collectScalars(rendered).includes("admin.user.secrets.deleted:github_token"));
});

test("secrets panel exposes loading and sanitized failure states", () => {
  const loading = createHarness({ query: { isLoading: true, error: null } }).render();
  assert.ok(collectScalars(loading).includes("admin.user.secrets.loading"));

  const failed = createHarness({
    mutationError: { message: "request failed" },
  }).render();
  assert.ok(
    collectScalars(failed).includes("admin.user.secrets.actionFailed:request failed"),
  );
});
