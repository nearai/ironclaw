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
  onAlways,
  onApprove,
  onDeny,
} = {}) {
  let stateCalls = 0;
  const effects = [];
  const expandedPayloadUpdates = [];
  const resolvingUpdates = [];
  const refs = [];
  const context = {
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    React: {
      useCallback: (fn) => fn,
      useEffect: (fn, deps) => {
        effects.push({ fn, deps });
      },
      useMemo: (fn) => fn(),
      useRef: (initial) => {
        const ref = { current: initial };
        refs.push(ref);
        return ref;
      },
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
        if (stateCalls === 3) {
          return [
            typeof initial === "function" ? initial() : initial,
            (value) => {
              resolvingUpdates.push(value);
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
  const rendered = context.globalThis.__testExports.ApprovalCard({
    gate,
    onAlways,
    onApprove,
    onDeny,
  });
  return { rendered, effects, expandedPayloadUpdates, resolvingUpdates, refs };
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

function findPrimaryOnClick(node) {
  if (!node || typeof node !== "object") return null;
  if (
    Array.isArray(node.strings) &&
    node.strings.some((value) => value.includes('variant="primary" onClick='))
  ) {
    const index = node.strings.findIndex((value) =>
      value.includes('variant="primary" onClick='),
    );
    return node.values[index];
  }
  for (const value of node.values || []) {
    const found = findPrimaryOnClick(value);
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

test("ApprovalCard leaves a newer gate resolving when an older resolution finishes", async () => {
  let finishApproval;
  const gate = defaultApprovalGate();
  const { rendered, refs, resolvingUpdates } = renderApprovalCard({
    gate,
    onApprove,
  });
  const primaryOnClick = findPrimaryOnClick(rendered);

  assert.equal(typeof primaryOnClick, "function");
  const resolution = primaryOnClick();
  assert.deepEqual(resolvingUpdates, [true]);

  const currentGateRef = refs[1];
  currentGateRef.current = { ...gate, toolName: "builtin.other" };
  refs[0].current = true;
  resolvingUpdates.push(true);
  finishApproval();
  await resolution;

  assert.deepEqual(resolvingUpdates, [true, true]);

  function onApprove() {
    return new Promise((resolve) => {
      finishApproval = resolve;
    });
  }
});
