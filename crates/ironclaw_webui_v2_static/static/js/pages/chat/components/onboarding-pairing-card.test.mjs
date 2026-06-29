import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

import { channelConnectionDisplayName } from "../../../lib/channel-connection-events.js";

function onboardingPairingCardSourceForTest() {
  const source = readFileSync(new URL("./onboarding-pairing-card.js", import.meta.url), "utf8");
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
    lines.push(line.replace("export function OnboardingPairingCard", "function OnboardingPairingCard"));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { OnboardingPairingCard };`;
}

function findComponent(node, component) {
  if (!node || typeof node !== "object") return null;
  if (!Array.isArray(node.values)) return null;
  const componentIndex = node.values.indexOf(component);
  if (componentIndex >= 0) {
    return node;
  }
  for (const value of node.values) {
    const found = findComponent(value, component);
    if (found) return found;
  }
  return null;
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

test("OnboardingPairingCard renders configured error copy instead of raw submit errors", async () => {
  const stateUpdates = [];
  let stateIndex = 0;
  const context = {
    Button() {},
    channelConnectionDisplayName,
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    React: {
      useState: (initial) => {
        const index = stateIndex++;
        const initialValues = ["PAIRCODE", "", false];
        return [
          initialValues[index] ?? initial,
          (value) => stateUpdates.push({ index, value }),
        ];
      },
    },
  };

  vm.runInNewContext(onboardingPairingCardSourceForTest(), context);
  const rendered = context.globalThis.__testExports.OnboardingPairingCard({
    onboarding: {
      extensionName: "slack",
      errorMessage: "Invalid or expired Slack pairing code. Run /pair in Slack to get a new one.",
    },
    onSubmit: async () => {
      throw new Error("raw backend path /secret/token");
    },
  });

  const button = findComponent(rendered, context.Button);
  const props = componentProps(button, context.Button);
  await props.onClick();

  assert.deepEqual(
    stateUpdates.filter((update) => update.index === 1).map((update) => update.value),
    ["", "Invalid or expired Slack pairing code. Run /pair in Slack to get a new one."],
  );
});
