import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function activityRunSourceForTest() {
  const source = readFileSync(new URL("./activity-run.js", import.meta.url), "utf8");
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
    lines.push(line.replace("export function ActivityRun", "function ActivityRun"));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { ActivityRun };`;
}

test("ActivityRun auto-expands running tool activity", () => {
  const context = {
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    Icon() {},
    MarkdownRenderer() {},
    React: {
      useEffect: () => {},
      useState: (initial) => [typeof initial === "function" ? initial() : initial, () => {}],
    },
    summarizeActivity: () => ({
      label: "Activity - 1 tool, running",
      hasError: false,
    }),
    ToolActivity() {},
  };

  vm.runInNewContext(activityRunSourceForTest(), context);
  const tree = context.globalThis.__testExports.ActivityRun({
    activity: [
      {
        id: "tool-search",
        role: "tool_activity",
        toolName: "web-access.search",
        toolStatus: "running",
      },
    ],
  });

  assert.ok(tree.values.includes("true"));
  assert.ok(hasComponentNamed(tree, "ActivityItem"));
});

function hasComponentNamed(node, name) {
  if (!node || typeof node !== "object" || !Array.isArray(node.values)) return false;
  if (node.values.some((value) => typeof value === "function" && value.name === name)) {
    return true;
  }
  return node.values.some((value) => {
    if (Array.isArray(value)) return value.some((item) => hasComponentNamed(item, name));
    return hasComponentNamed(value, name);
  });
}
