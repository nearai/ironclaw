// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";
import "../../../test/vm-tsx-setup";

function authGenericCardSourceForTest() {
  const source = readFileSync(new URL("./auth-generic-card.tsx", import.meta.url), "utf8");
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
    lines.push(line.replace("export function AuthGenericCard", "function AuthGenericCard"));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { AuthGenericCard };`;
}

function walk(node, visit, seen = new Set()) {
  if (node == null || seen.has(node)) return;
  if (typeof node === "object") seen.add(node);
  visit(node);
  if (typeof node !== "object") return;
  for (const child of Array.isArray(node) ? node : Object.values(node)) {
    walk(child, visit, seen);
  }
}

test("AuthGenericCard never promises settings without an actionable settings route", () => {
  const translationKeys = [];
  const context = {
    globalThis: {},
    useT: () => (key) => {
      translationKeys.push(key);
      return key === "authGate.unsupportedChallengeNeutral"
        ? "This authentication method is not available in this view."
        : key;
    },
    Button() {},
    AuthGateShell() {},
  };
  vm.runInNewContext(authGenericCardSourceForTest(), context);
  const rendered = context.globalThis.__testExports.AuthGenericCard({
    gate: { challengeKind: "other" },
    onCancel() {},
  });
  assert.ok(
    translationKeys.includes("authGate.unsupportedChallengeNeutral"),
    "unsupported challenges must request the neutral fallback copy",
  );
  assert.ok(
    !translationKeys.includes("authGate.unsupportedChallenge"),
    "the retired settings-specific fallback key must not be requested",
  );

  let promisesSettings = false;
  let hasSettingsAction = false;
  walk(rendered, (value) => {
    if (typeof value === "string" && /open settings/i.test(value)) promisesSettings = true;
    if (
      value &&
      typeof value === "object" &&
      (String(value.href || "").includes("settings") ||
        String(value.to || "").includes("settings") ||
        String(value.route || "").includes("settings"))
    ) {
      hasSettingsAction = true;
    }
  });

  assert.ok(
    !promisesSettings || hasSettingsAction,
    "challenge kind Other must use neutral unsupported copy or render a real settings action",
  );
});
