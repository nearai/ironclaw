import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function sourceForTest(path, exportNames) {
  const source = readFileSync(new URL(path, import.meta.url), "utf8");
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
  return `${lines.join("\n")}\nglobalThis.__testExports = { ${exportNames.join(", ")} };`;
}

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

function collectTemplateText(root) {
  const text = [];
  visit(root, (node) => {
    if (Array.isArray(node.strings)) text.push(...node.strings);
  });
  return text.join("");
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

function findComponentNode(root, component) {
  let found = null;
  visit(root, (node) => {
    if (!found && Array.isArray(node.values) && node.values.includes(component)) {
      found = node;
    }
  });
  return found;
}

function findTemplateNode(root, fragment) {
  let found = null;
  visit(root, (node) => {
    if (
      !found &&
      Array.isArray(node.strings) &&
      node.strings.some((part) => part.includes(fragment))
    ) {
      found = node;
    }
  });
  return found;
}

function componentProps(node, component) {
  const props = {};
  const start = node.values.indexOf(component);
  for (let index = start + 1; index < node.values.length; index += 1) {
    const name = node.strings[index]?.match(/([A-Za-z][A-Za-z0-9]*)=\s*$/)?.[1];
    if (name) props[name] = node.values[index];
  }
  return props;
}

function renderToolsModule({ tools = [] } = {}) {
  const saved = [];
  const context = {
    Badge: "Badge",
    Card: "Card",
    Icon: "Icon",
    globalThis: {},
    html,
    matchesSearch: (query, values) =>
      !query || values.some((value) => String(value || "").includes(query)),
    useT: () => (key) => key,
    useTools: () => ({
      tools,
      query: { isLoading: false, error: null },
      setPermission: () => {},
      savedTools: {},
    }),
  };
  vm.runInNewContext(
    sourceForTest("./tools-tab.js", ["ToolsTab", "AutoApproveCard", "Switch", "ToolRow"]),
    context
  );
  return { exports: context.globalThis.__testExports, saved };
}

test("Tools tab renders global auto-approve control and saves the operator key", () => {
  const { exports, saved } = renderToolsModule();
  const rendered = exports.AutoApproveCard({
    settings: { "agent.auto_approve_tools": false },
    savedKeys: {},
    onSave: (key, value) => saved.push({ key, value }),
  });

  assert.match(collectTemplateText(exports.Switch({ checked: false, label: "x", onChange: () => {} })), /role="switch"/);
  const switchNode = findComponentNode(rendered, exports.Switch);
  assert.ok(switchNode, "expected auto-approve card to render a switch");

  componentProps(switchNode, exports.Switch).onChange(true);
  assert.deepEqual(saved, [{ key: "agent.auto_approve_tools", value: true }]);
});

test("Auto-approve toggle defaults ON when unset and reads present values strictly", () => {
  const { exports } = renderToolsModule();
  const checkedFor = (settings) => {
    const rendered = exports.AutoApproveCard({ settings, savedKeys: {}, onSave: () => {} });
    const node = findComponentNode(rendered, exports.Switch);
    return componentProps(node, exports.Switch).checked;
  };
  // Absent → default ON.
  assert.equal(checkedFor({}), true, "unset must default ON");
  // Present values read strictly.
  assert.equal(checkedFor({ "agent.auto_approve_tools": true }), true);
  assert.equal(checkedFor({ "agent.auto_approve_tools": "true" }), true);
  assert.equal(checkedFor({ "agent.auto_approve_tools": false }), false);
  // Unexpected falsy must read OFF, not silently ON.
  assert.equal(checkedFor({ "agent.auto_approve_tools": 0 }), false, "0 must read OFF");
  assert.equal(checkedFor({ "agent.auto_approve_tools": "" }), false, "empty string must read OFF");
});

test("Tool permission select follows global unless a per-tool override exists", () => {
  const { exports } = renderToolsModule();
  const globalTool = exports.ToolRow({
    tool: {
      name: "builtin.echo",
      description: "Echo",
      state: "always_allow",
      default_state: "ask_each_time",
      effective_source: "global",
      locked: false,
    },
    onPermissionChange: () => {},
    isSaved: false,
  });
  const globalSelect = findTemplateNode(globalTool, "<select");
  assert.equal(globalSelect.values[0], "default");
  assert.ok(collectScalars(globalTool).includes("tools.followDefault"));

  const overrideTool = exports.ToolRow({
    tool: {
      name: "builtin.echo",
      description: "Echo",
      state: "ask_each_time",
      default_state: "ask_each_time",
      effective_source: "override",
      locked: false,
    },
    onPermissionChange: () => {},
    isSaved: false,
  });
  const overrideSelect = findTemplateNode(overrideTool, "<select");
  assert.equal(overrideSelect.values[0], "ask_each_time");
});
