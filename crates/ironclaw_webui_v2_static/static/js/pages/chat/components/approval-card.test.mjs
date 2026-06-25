import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function approvalCardSourceForTest() {
  const source = readFileSync(new URL("./approval-card.js", import.meta.url), "utf8");
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
    lines.push(
      line.replace("export function ApprovalCard", "function ApprovalCard"),
    );
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { ApprovalCard };`;
}

function renderApprovalCard({
  expandedPayload = false,
  gate = defaultApprovalGate(),
  globalAutoApproveEnabled = null,
} = {}) {
  let stateCalls = 0;
  const effects = [];
  const expandedPayloadUpdates = [];
  const context = {
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    React: {
      useCallback: (fn) => fn,
      useEffect: (fn, deps) => {
        effects.push({ fn, deps });
      },
      useMemo: (fn) => fn(),
      useState: (initial) => {
        stateCalls += 1;
        if (stateCalls === 2) {
          return [
            expandedPayload,
            (value) => {
              expandedPayloadUpdates.push(value);
            },
          ];
        }
        return [typeof initial === "function" ? initial() : initial, () => {}];
      },
    },
    useT: () => (key) => (
      {
        "approval.approve": "Approve",
        "approval.deny": "Deny",
        "approval.globalAutoApproveLink": "Automatically approve and execute all actions",
        "approval.showCommandPreview": "Show preview",
        "approval.thisTool": "this tool",
        "approval.title": "Approval required",
        "approval.viewFullCommand": "View full command",
      }[key] || key
    ),
    Button() {},
    Badge() {},
    Icon() {},
    Link() {},
    classifyRisk: () => ({ key: "tool.riskExec", tone: "danger" }),
  };
  vm.runInNewContext(approvalCardSourceForTest(), context);
  const rendered = context.globalThis.__testExports.ApprovalCard({
    gate,
    globalAutoApproveEnabled,
  });
  return { rendered, effects, expandedPayloadUpdates, Link: context.Link };
}

function defaultApprovalGate() {
  return {
    toolName: "builtin.shell",
    description: "Run shell command",
    allowAlways: false,
    approvalDetails: [
      { label: "Action", value: "Run command" },
      { label: "Command", value: "python -c " + "x".repeat(640) },
    ],
  };
}

function collectStrings(node, output = []) {
  if (typeof node === "string") {
    output.push(node);
    return output;
  }
  if (Array.isArray(node)) {
    for (const item of node) collectStrings(item, output);
    return output;
  }
  if (!node || typeof node !== "object") return output;
  collectStrings(node.strings, output);
  collectStrings(node.values, output);
  return output;
}

function findComponent(node, component) {
  if (!node || typeof node !== "object") return null;
  if (!Array.isArray(node.values)) return null;
  if (node.values.includes(component)) return node;
  for (const value of node.values) {
    const found = findComponent(value, component);
    if (found) return found;
  }
  return null;
}

test("ApprovalCard truncates long command details by default", () => {
  const { rendered } = renderApprovalCard();
  const text = collectStrings(rendered).join("\n");

  assert.match(text, /View full command/);
  assert.match(text, /python -c x{470}/);
  assert.match(text, /\n\.\.\./);
  assert.doesNotMatch(text, new RegExp(`python -c ${"x".repeat(640)}`));
});

test("ApprovalCard can render full long command details when expanded", () => {
  const { rendered } = renderApprovalCard({ expandedPayload: true });
  const text = collectStrings(rendered).join("\n");

  assert.match(text, /Show preview/);
  assert.match(text, new RegExp(`python -c ${"x".repeat(640)}`));
});

test("ApprovalCard ignores long parameters when approval details are rendered", () => {
  const { rendered } = renderApprovalCard({
    gate: {
      toolName: "builtin.shell",
      description: "Run shell command",
      parameters: "python -c " + "x".repeat(640),
      allowAlways: false,
      approvalDetails: [
        { label: "Action", value: "Run command" },
        { label: "Command", value: "echo ok" },
      ],
    },
  });
  const text = collectStrings(rendered).join("\n");

  assert.doesNotMatch(text, /View full command/);
  assert.doesNotMatch(text, new RegExp(`python -c ${"x".repeat(640)}`));
  assert.match(text, /echo ok/);
});

test("ApprovalCard resets expanded command details when the gate changes", () => {
  const gate = defaultApprovalGate();
  const { rendered, effects, expandedPayloadUpdates } = renderApprovalCard({
    expandedPayload: true,
    gate,
  });
  const text = collectStrings(rendered).join("\n");

  assert.match(text, /Show preview/);
  assert.equal(effects.length, 1);
  assert.equal(effects[0].deps.length, 1);
  assert.equal(effects[0].deps[0].toolName, gate.toolName);
  assert.equal(effects[0].deps[0].approvalDetails[1].value, gate.approvalDetails[1].value);

  effects[0].fn();
  assert.deepEqual(expandedPayloadUpdates, [false]);
});

test("ApprovalCard links to tool settings when global auto-approve is off", () => {
  const { rendered, Link } = renderApprovalCard({
    gate: { ...defaultApprovalGate(), allowAlways: true },
    globalAutoApproveEnabled: false,
  });
  const text = collectStrings(rendered).join("\n");
  const link = findComponent(rendered, Link);

  assert.match(text, /Automatically approve and execute all actions/);
  assert.ok(link, "global auto-approve link should render");
  assert.ok(link.strings.some((part) => part.includes('to="/settings/tools"')));
});

test("ApprovalCard hides the global settings link when global auto-approve is on", () => {
  const { rendered, Link } = renderApprovalCard({
    gate: { ...defaultApprovalGate(), allowAlways: true },
    globalAutoApproveEnabled: true,
  });
  const text = collectStrings(rendered).join("\n");

  assert.doesNotMatch(text, /Automatically approve and execute all actions/);
  assert.equal(findComponent(rendered, Link), null);
});
