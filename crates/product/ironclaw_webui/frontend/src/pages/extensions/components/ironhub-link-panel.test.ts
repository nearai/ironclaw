// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

function panelSourceForTest() {
  const source = readFileSync(new URL("./ironhub-link-panel.tsx", import.meta.url), "utf8");
  const lines = [];
  let skippingImport = false;
  for (const line of source.split("\n")) {
    if (!skippingImport && line.startsWith("import ")) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    if (skippingImport) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { IronhubLinkPanel };`;
}

function renderPanel(query, options = {}) {
  const captured = options.captured ?? {};
  captured.mutations = [];
  captured.sentKeys = [];
  captured.buttons = [];
  const context = {
    Card() { return null; },
    Button(props) {
      captured.buttons.push(props);
      return null;
    },
    Icon() {},
    React: { useState: () => [options.key ?? "", () => {}] },
    useQuery: (queryOptions) => {
      captured.query = queryOptions;
      return query;
    },
    useMutation: (mutationOptions) => {
      captured.mutations.push(mutationOptions);
      return {
        mutate: () => {
          captured.mutated = mutationOptions;
        },
        isPending:
          typeof options.pending === "function"
            ? options.pending(captured.mutations.length)
            : (options.pending ?? false),
        error: null,
      };
    },
    useQueryClient: () => ({ setQueryData: () => {} }),
    console: { log: () => {} },
    getIronhubLink: () => {},
    setIronhubSharedKey: (value) => {
      captured.sentKeys.push(value);
    },
    clearIronhubSharedKey: () => {},
    globalThis: {},
    html(strings, ...values) {
      return { strings: Array.from(strings), values };
    },
    useT: () => (key) => key,
  };
  vm.runInNewContext(panelSourceForTest(), context);
  return context.globalThis.__testExports.IronhubLinkPanel();
}

function renderedText(node, found = []) {
  if (Array.isArray(node)) {
    for (const item of node) renderedText(item, found);
    return found;
  }
  if (!node || typeof node !== "object") {
    if (typeof node === "string") found.push(node);
    return found;
  }
  if (Array.isArray(node.strings)) found.push(...node.strings);
  if (Array.isArray(node.values)) {
    for (const value of node.values) renderedText(value, found);
  }
  return found;
}

test("a 403 hides the panel entirely rather than showing an error", () => {
  const rendered = renderPanel({
    isPending: false,
    error: { status: 403 },
    data: undefined,
  });
  assert.equal(rendered, null, "a non-operator must not see the panel at all");
});

test("a non-403 failure surfaces an error instead of vanishing silently", () => {
  const rendered = renderPanel({
    isPending: false,
    error: { status: 500 },
    data: undefined,
  });
  assert.notEqual(rendered, null, "a real failure must not be swallowed");
  assert.ok(
    renderedText(rendered).some((text) => text.includes("ironhub.link.loadFailed")),
    "the operator must be told the details could not load",
  );
});

test("while loading, the panel renders nothing", () => {
  const rendered = renderPanel({ isPending: true, error: null, data: undefined });
  assert.equal(rendered, null);
});

test("a stored but inactive key raises the restart notice", () => {
  const rendered = renderPanel({
    isPending: false,
    error: null,
    data: {
      register_url: "https://agent.example.com/api/ironhub/register",
      key_stored: true,
      key_active: false,
    },
  });
  const text = renderedText(rendered);
  assert.ok(
    text.some((entry) => entry.includes("settings.restartRequired")),
    "stored-but-not-active is the restart-pending state",
  );
  assert.ok(text.some((entry) => entry.includes("ironhub.link.stateStored")));
});

test("an active key shows no restart notice", () => {
  const rendered = renderPanel({
    isPending: false,
    error: null,
    data: {
      register_url: "https://agent.example.com/api/ironhub/register",
      key_stored: false,
      key_active: true,
    },
  });
  const text = renderedText(rendered);
  assert.ok(!text.some((entry) => entry.includes("settings.restartRequired")));
  assert.ok(text.some((entry) => entry.includes("ironhub.link.stateActive")));
});

test("an env override explains itself and promises no restart", () => {
  const rendered = renderPanel({
    isPending: false,
    error: null,
    data: {
      register_url: "https://agent.example.com/api/ironhub/register",
      key_stored: true,
      key_active: false,
      env_override: true,
    },
  });
  const text = renderedText(rendered);
  assert.ok(
    text.some((entry) => entry.includes("ironhub.link.envOverride")),
    "the operator must be told the environment variable wins",
  );
  assert.ok(
    !text.some((entry) => entry.includes("settings.restartRequired")),
    "no restart would promote the stored key, so none may be promised",
  );
});

test("the remove affordance appears only when a key is stored", () => {
  const stored = renderedText(
    renderPanel({
      isPending: false,
      error: null,
      data: { register_url: null, key_stored: true, key_active: false },
    }),
  );
  assert.ok(
    stored.some((entry) => entry.includes("ironhub.link.clearKey")),
    "a stored key must be removable",
  );

  const absent = renderedText(
    renderPanel({
      isPending: false,
      error: null,
      data: { register_url: null, key_stored: false, key_active: false },
    }),
  );
  assert.ok(
    !absent.some((entry) => entry.includes("ironhub.link.clearKey")),
    "there is nothing to remove when no key is stored",
  );
});

test("the panel never renders key material", () => {
  const rendered = renderPanel({
    isPending: false,
    error: null,
    data: {
      register_url: "https://agent.example.com/api/ironhub/register",
      key_stored: true,
      key_active: true,
      shared_key: "ihub_sk_TestSharedKey00000000000000000000",
    },
  });
  const text = renderedText(rendered).join(" ");
  assert.ok(!text.includes("ihub_sk_TestSharedKey"), "no key material may reach the DOM");
});

test("the shared key is trimmed before it reaches the API", () => {
  const captured = {};
  renderPanel(
    {
      isPending: false,
      error: null,
      data: {
        register_url: "https://agent.example.com/api/ironhub/register",
        key_stored: false,
        key_active: false,
      },
    },
    { captured, key: `  ${"k".repeat(40)}\n` },
  );

  const save = captured.mutations[0];
  save.mutationFn();

  assert.deepEqual(
    captured.sentKeys,
    ["k".repeat(40)],
    "a pasted key with surrounding whitespace must be stored as the validated value",
  );
});

test("the link query retries once and never retries a 403", () => {
  const captured = {};
  renderPanel(
    {
      isPending: false,
      error: null,
      data: {
        register_url: "https://agent.example.com/api/ironhub/register",
        key_stored: false,
        key_active: false,
      },
    },
    { captured },
  );

  const { retry } = captured.query;
  assert.equal(retry(0, { status: 500 }), true, "the first failure retries");
  assert.equal(retry(1, { status: 500 }), false, "a second failure gives up");
  assert.equal(retry(0, { status: 403 }), false, "a non-operator is never retried");
});

test("a save in flight disables the clear control too", () => {
  const captured = {};
  const panel = renderPanel(
    {
      isPending: false,
      error: null,
      data: {
        register_url: "https://agent.example.com/api/ironhub/register",
        key_stored: true,
        key_active: false,
      },
    },
    // The save mutation is registered first, so fn(1) = true marks it pending
    // while the clear mutation (fn(2)) stays idle.
    { captured, key: "k".repeat(40), pending: (index) => index === 1 },
  );

  const buttons = [];
  walkTree(panel, (node) => {
    if (node && node.type && node.props) buttons.push(node);
  });
  const isButton = (node) =>
    typeof node.type === "function" && node.type.name === "Button";
  const saveBtn = buttons.find(
    (node) => isButton(node) && node.props.disabled !== undefined,
  );
  const clearBtn = buttons.find(
    (node) => isButton(node) && node.props.variant === "ghost",
  );

  assert.ok(saveBtn && clearBtn, "both controls are rendered for a stored key");
  assert.equal(
    clearBtn.props.disabled,
    true,
    "a pending save must disable the clear button",
  );
});

test("a clear in flight disables the save control too", () => {
  const captured = {};
  const panel = renderPanel(
    {
      isPending: false,
      error: null,
      data: {
        register_url: "https://agent.example.com/api/ironhub/register",
        key_stored: true,
        key_active: false,
      },
    },
    // Clear is the second mutation registered; mark it pending only.
    { captured, key: "k".repeat(40), pending: (index) => index === 2 },
  );

  const buttons = [];
  walkTree(panel, (node) => {
    if (node && node.type && node.props) buttons.push(node);
  });
  const isButton = (node) =>
    typeof node.type === "function" && node.type.name === "Button";
  const saveBtn = buttons.find(
    (node) => isButton(node) && node.props.disabled !== undefined,
  );
  const clearBtn = buttons.find(
    (node) => isButton(node) && node.props.variant === "ghost",
  );

  assert.ok(saveBtn && clearBtn, "both controls are rendered for a stored key");
  assert.equal(
    saveBtn.props.disabled,
    true,
    "a pending clear must disable the save button",
  );
  assert.equal(
    clearBtn.props.disabled,
    true,
    "a pending save must disable the clear button",
  );
});


function walkTree(node, visit) {
  if (!node || typeof node !== "object") return;
  visit(node);
  if (Array.isArray(node)) {
    for (const child of node) walkTree(child, visit);
    return;
  }
  const children =
    node.children ??
    (Array.isArray(node.values) ? node.values : null);
  if (children) walkTree(children, visit);
}
