// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

import { channelConnectionDisplayName } from "../../../lib/channel-connection-events";

function sourceForTest() {
  const source = readFileSync(new URL("./onboarding-pairing-card.tsx", import.meta.url), "utf8");
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
  if (!node || typeof node !== "object" || !Array.isArray(node.values)) return null;
  if (node.values.includes(component)) return node;
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

function tForTest(key, params = {}) {
  const values = {
    "common.dismiss": "Dismiss",
    "connection.connecting": "Connecting...",
    "pairing.connectFailedRetry": "Connection failed. Try again.",
    "pairing.connect": "Connect",
    "pairing.connectFromExtensions": `Connect ${params.name} from Extensions.`,
    "pairing.connectTitle": `Connect ${params.name}`,
    "pairing.connectInstructions": `Follow the connection steps for ${params.name} to continue.`,
  };
  return values[key] || key;
}

function renderCard(props, stateValues = []) {
  let stateIndex = 0;
  const updates = [];
  const context = {
    Button() {},
    PairingWebCodePanel() {},
    channelConnectionDisplayName,
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    React: {
      useState: (initial) => {
        const index = stateIndex++;
        return [
          stateValues[index] ?? initial,
          (value) => updates.push({ index, value }),
        ];
      },
    },
    useT: () => tForTest,
  };
  vm.runInNewContext(sourceForTest(), context);
  return {
    Button: context.Button,
    PairingWebCodePanel: context.PairingWebCodePanel,
    rendered: context.globalThis.__testExports.OnboardingPairingCard(props),
    updates,
  };
}

test("web_generated_code renders the generic host-issued code panel", () => {
  const view = renderCard({
    onboarding: {
      extensionName: "acme-messenger",
      strategy: "web_generated_code",
      instructions: "Open Acme with the generated link.",
    },
  });
  const panel = findComponent(view.rendered, view.PairingWebCodePanel);
  assert.ok(panel);
  assert.deepEqual(componentProps(panel, view.PairingWebCodePanel), {
    compact: true,
    extensionId: "acme-messenger",
    displayName: "Acme Messenger",
  });
  assert.ok(!JSON.stringify(view.rendered).includes("<input"));
});

test("oauth renders the generic configure action and keeps it busy after start", async () => {
  let started = 0;
  const view = renderCard(
    {
      onboarding: { extensionName: "workspace-chat", strategy: "oauth" },
      onConfigure: async () => {
        started += 1;
      },
    },
    ["", false, null],
  );
  const button = findComponent(view.rendered, view.Button);
  await componentProps(button, view.Button).onClick();
  assert.equal(started, 1);
  assert.deepEqual(
    view.updates.filter(({ index }) => index === 1).map(({ value }) => value),
    [true],
  );
});

test("admin-managed channels show guidance without a credential input", () => {
  const view = renderCard({
    onboarding: {
      extensionName: "tenant-channel",
      strategy: "admin_managed_channels",
      instructions: "Ask an administrator to enable this channel.",
    },
  });
  const body = JSON.stringify(view.rendered);
  assert.match(body, /Ask an administrator/);
  assert.match(body, /Connect Tenant Channel from Extensions/);
  assert.ok(!body.includes("<input"));
});
