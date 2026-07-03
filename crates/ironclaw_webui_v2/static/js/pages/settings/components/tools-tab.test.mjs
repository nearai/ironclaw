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

function renderToolsModule({ tools = [], translations = {} } = {}) {
  const saved = [];
  const context = {
    Badge: "Badge",
    Card: "Card",
    Icon: "Icon",
    globalThis: {},
    html,
    matchesSearch: (query, values) =>
      !query || values.some((value) => String(value || "").includes(query)),
    useT: () => (key) => translations[key] || key,
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

test("Tool rows localize built-in descriptions by capability id", () => {
  const { exports } = renderToolsModule({
    translations: {
      "tools.description.builtin.echo": "回显一条消息",
    },
  });

  const rendered = exports.ToolRow({
    tool: {
      name: "builtin.echo",
      description: "Echo a message",
      state: "always_allow",
      default_state: "ask_each_time",
      effective_source: "global",
      locked: false,
    },
    onPermissionChange: () => {},
    isSaved: false,
  });

  const scalars = collectScalars(rendered);
  assert.ok(scalars.includes("回显一条消息"));
  assert.ok(!scalars.includes("Echo a message"));
});

test("Tool rows localize descriptions when backend payload omits description", () => {
  const { exports } = renderToolsModule({
    translations: {
      "tools.description.builtin.echo": "回显一条消息",
    },
  });

  const rendered = exports.ToolRow({
    tool: {
      name: "builtin.echo",
      state: "always_allow",
      default_state: "ask_each_time",
      effective_source: "global",
      locked: false,
    },
    onPermissionChange: () => {},
    isSaved: false,
  });

  assert.ok(collectScalars(rendered).includes("回显一条消息"));
});

test("Tool rows localize extension and provider capability descriptions", () => {
  const { exports } = renderToolsModule({
    translations: {
      "tools.description.builtin.extension_search": "搜索本地 Reborn 扩展目录",
      "tools.description.nearai.web_search": "通过 NEAR AI MCP 服务器搜索",
    },
  });
  const renderDescription = (name) =>
    collectScalars(
      exports.ToolRow({
        tool: {
          name,
          description: "Backend description",
          state: "always_allow",
          default_state: "ask_each_time",
          effective_source: "global",
          locked: false,
        },
        onPermissionChange: () => {},
        isSaved: false,
      })
    );

  assert.ok(renderDescription("builtin.extension_search").includes("搜索本地 Reborn 扩展目录"));
  assert.ok(renderDescription("nearai.web_search").includes("通过 NEAR AI MCP 服务器搜索"));
});

test("Tools tab search matches localized and raw tool descriptions", () => {
  const tools = [
    {
      name: "builtin.echo",
      description: "Echo a message",
      state: "always_allow",
      default_state: "ask_each_time",
      effective_source: "global",
      locked: false,
    },
  ];
  const { exports } = renderToolsModule({
    tools,
    translations: {
      "tools.description.builtin.echo": "回显一条消息",
    },
  });

  const zhRendered = exports.ToolsTab({ searchQuery: "回显" });
  assert.ok(
    findComponentNode(zhRendered, exports.ToolRow),
    "localized description should keep the tool visible"
  );

  const enRendered = exports.ToolsTab({ searchQuery: "Echo" });
  assert.ok(
    findComponentNode(enRendered, exports.ToolRow),
    "raw backend description should still keep the tool visible"
  );
});

test("Tools tab search does not index locked tools as disabled unless disabled", () => {
  const tools = [
    {
      name: "builtin.echo",
      description: "Echo a message",
      state: "always_allow",
      default_state: "ask_each_time",
      effective_source: "global",
      locked: true,
    },
  ];
  const { exports } = renderToolsModule({
    tools,
    translations: {
      "tools.disabled": "disabled",
    },
  });

  const rendered = exports.ToolsTab({ searchQuery: "disabled" });
  assert.equal(findComponentNode(rendered, exports.ToolRow), null);
  assert.ok(collectScalars(rendered).includes("tools.noMatch"));
});
