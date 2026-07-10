import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function budgetTabHelpers() {
  const source = readFileSync(new URL("./budget-tab.js", import.meta.url), "utf8");
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
      line
        .replace("export function formatUsd", "function formatUsd")
        .replace("export function formatPercent", "function formatPercent")
        .replace("export function BudgetTab", "function BudgetTab"),
    );
  }
  const context = { globalThis: {} };
  vm.runInNewContext(
    `${lines.join("\n")}\nglobalThis.__testExports = { formatUsd, formatPercent };`,
    context,
  );
  return context.globalThis.__testExports;
}

test("budget tab formats compact money values", () => {
  const { formatUsd } = budgetTabHelpers();

  assert.equal(formatUsd("0"), "$0");
  assert.equal(formatUsd("12.5000"), "$12.5");
  assert.equal(formatUsd("1234.56789"), "$1,234.5679");
  assert.equal(formatUsd(null, "Unlimited"), "Unlimited");
});

test("budget tab formats utilization percentages", () => {
  const { formatPercent } = budgetTabHelpers();

  assert.equal(formatPercent(0), "0.0%");
  assert.equal(formatPercent(0.901), "90.1%");
  assert.equal(formatPercent(null), "-");
});
