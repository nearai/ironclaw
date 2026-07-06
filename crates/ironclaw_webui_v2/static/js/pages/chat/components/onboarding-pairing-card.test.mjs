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
        const initialValues = ["PAIRCODE", "", "idle"];
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

test("OnboardingPairingCard shows connection-succeeded copy when the resume faults, not the invalid-code error", async () => {
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
        const initialValues = ["PAIRCODE", "", "idle"];
        return [
          initialValues[index] ?? initial,
          (value) => stateUpdates.push({ index, value }),
        ];
      },
    },
  };

  vm.runInNewContext(onboardingPairingCardSourceForTest(), context);
  const rendered = context.globalThis.__testExports.OnboardingPairingCard({
    onboarding: { extensionName: "slack" },
    onSubmit: async () => {
      // Mirrors submitChannelConnectionPairing throwing on resume_error: the
      // binding is durable (connected), but the parked turn didn't resume.
      const error = new Error("channel connection resume did not complete");
      error.resumeFailed = true;
      throw error;
    },
  });

  const button = findComponent(rendered, context.Button);
  const props = componentProps(button, context.Button);
  await props.onClick();

  const errorUpdates = stateUpdates.filter((update) => update.index === 1).map((u) => u.value);
  // Cleared first, then the resume-specific message — never the invalid-code copy.
  assert.equal(errorUpdates[0], "");
  assert.match(errorUpdates[1], /connected, but this chat couldn't continue/i);
  // And it drops back to idle (spinner off) rather than hanging on "resuming".
  assert.deepEqual(
    stateUpdates.filter((update) => update.index === 2).map((u) => u.value),
    ["submitting", "idle"],
  );
});

test("OnboardingPairingCard holds a connecting state after a successful pair instead of resetting to idle", async () => {
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
        const initialValues = ["PAIRCODE", "", "idle"];
        return [
          initialValues[index] ?? initial,
          (value) => stateUpdates.push({ index, value }),
        ];
      },
    },
  };

  vm.runInNewContext(onboardingPairingCardSourceForTest(), context);
  const rendered = context.globalThis.__testExports.OnboardingPairingCard({
    onboarding: { extensionName: "slack" },
    onSubmit: async () => ({ success: true }),
  });

  const button = findComponent(rendered, context.Button);
  const props = componentProps(button, context.Button);
  await props.onClick();

  // The parked turn resumes asynchronously — the gate (and this card) clear a
  // beat later via SSE. The status must move submitting → resuming and never
  // snap back to idle, or a successful pair reads as "nothing happened".
  assert.deepEqual(
    stateUpdates.filter((update) => update.index === 2).map((update) => update.value),
    ["submitting", "resuming"],
  );
  // The input clears on success.
  assert.deepEqual(
    stateUpdates.filter((update) => update.index === 0).map((update) => update.value),
    [""],
  );
});

test("OnboardingPairingCard shows a spinner and disables submit while busy, not while idle", () => {
  const renderWithStatus = (status) => {
    let stateIndex = 0;
    const context = {
      Button() {},
      channelConnectionDisplayName,
      globalThis: {},
      html: (strings, ...values) => ({ strings: Array.from(strings), values }),
      React: {
        useState: () => {
          const index = stateIndex++;
          return [["PAIRCODE", "", status][index], () => {}];
        },
      },
    };
    vm.runInNewContext(onboardingPairingCardSourceForTest(), context);
    const rendered = context.globalThis.__testExports.OnboardingPairingCard({
      onboarding: { extensionName: "slack", submittingLabel: "Connecting..." },
      onSubmit: async () => ({ success: true }),
    });
    const button = findComponent(rendered, context.Button);
    return { rendered, props: componentProps(button, context.Button) };
  };

  const idle = renderWithStatus("idle");
  assert.equal(idle.props.disabled, false);
  assert.ok(!JSON.stringify(idle.rendered).includes("animate-spin"), "idle shows no spinner");
  assert.ok(!JSON.stringify(idle.rendered).includes("Connecting..."), "idle shows submit label");

  for (const status of ["submitting", "resuming"]) {
    const busy = renderWithStatus(status);
    assert.equal(busy.props.disabled, true, `${status} disables submit`);
    assert.ok(JSON.stringify(busy.rendered).includes("animate-spin"), `${status} renders a spinner`);
    assert.ok(JSON.stringify(busy.rendered).includes("Connecting..."), `${status} shows connecting label`);
  }
});
