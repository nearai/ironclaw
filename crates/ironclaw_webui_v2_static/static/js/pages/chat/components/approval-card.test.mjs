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

function renderApprovalCard({ expandedPayload = false, gate = defaultApprovalGate() } = {}) {
  let stateCalls = 0;
  const context = {
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    React: {
      useCallback: (fn) => fn,
      useMemo: (fn) => fn(),
      useState: (initial) => {
        stateCalls += 1;
        if (stateCalls === 2) return [expandedPayload, () => {}];
        return [typeof initial === "function" ? initial() : initial, () => {}];
      },
    },
    useT: () => (key) => (
      {
        "approval.approve": "Approve",
        "approval.deny": "Deny",
        "approval.showCommandPreview": "Show preview",
        "approval.thisTool": "this tool",
        "approval.title": "Approval required",
        "approval.viewFullCommand": "View full command",
      }[key] || key
    ),
    Button() {},
    Badge() {},
    Icon() {},
    classifyRisk: () => ({ key: "tool.riskExec", tone: "danger" }),
  };
  vm.runInNewContext(approvalCardSourceForTest(), context);
  return context.globalThis.__testExports.ApprovalCard({
    gate,
  });
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

test("ApprovalCard truncates long command details by default", () => {
  const rendered = renderApprovalCard();
  const text = collectStrings(rendered).join("\n");

  assert.match(text, /View full command/);
  assert.match(text, /python -c x{470}/);
  assert.match(text, /\n\.\.\./);
  assert.doesNotMatch(text, new RegExp(`python -c ${"x".repeat(640)}`));
});

test("ApprovalCard can render full long command details when expanded", () => {
  const rendered = renderApprovalCard({ expandedPayload: true });
  const text = collectStrings(rendered).join("\n");

  assert.match(text, /Show preview/);
  assert.match(text, new RegExp(`python -c ${"x".repeat(640)}`));
});

test("ApprovalCard ignores long parameters when approval details are rendered", () => {
  const rendered = renderApprovalCard({
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
