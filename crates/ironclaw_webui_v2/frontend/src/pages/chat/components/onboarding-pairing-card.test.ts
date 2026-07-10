// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

import { channelConnectionDisplayName } from "../../../lib/channel-connection-events";

function onboardingPairingCardSourceForTest() {
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

function tForTest(key, params = {}) {
  const values = {
    "common.cancel": "Cancel",
    "common.dismiss": "Dismiss",
    "connection.connecting": "Connecting...",
    "pairing.checkCodeAndRetry": "Pairing failed. Check the code and try again.",
    "pairing.connect": "Connect",
    "pairing.connectTitle": `Connect ${params.name}`,
    "pairing.openAndPaste": `Open ${params.name}, get the pairing code, and paste it here.`,
    "pairing.placeholder": "PAIRING-CODE",
    "pairing.resumeFailed": `${params.name} connected, but this chat couldn't continue. Reload the page to keep going.`,
  };
  return values[key] || key;
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
    useT: () => tForTest,
  };

  vm.runInNewContext(onboardingPairingCardSourceForTest(), context);
  const rendered = context.globalThis.__testExports.OnboardingPairingCard({
    onboarding: {
      extensionName: "telegram",
      errorMessage: "Invalid or expired Telegram pairing code. Message the bot to get a new one.",
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
    ["", "Invalid or expired Telegram pairing code. Message the bot to get a new one."],
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
    useT: () => tForTest,
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
    useT: () => tForTest,
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
    useT: () => tForTest,
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
      useT: () => tForTest,
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
  assert.equal(idle.props.loading, false, "idle is not loading");
  assert.ok(!JSON.stringify(idle.rendered).includes("Connecting..."), "idle shows submit label");

  for (const status of ["submitting", "resuming"]) {
    const busy = renderWithStatus(status);
    // The shared Button owns the spinner + disabled state via `loading`.
    assert.equal(busy.props.loading, true, `${status} puts the submit button in loading`);
    assert.ok(JSON.stringify(busy.rendered).includes("Connecting..."), `${status} shows connecting label`);
  }
});

test("OnboardingPairingCard shows a spinner while OAuth configuration is waiting", () => {
  let stateIndex = 0;
  const context = {
    Button() {},
    channelConnectionDisplayName,
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    React: {
      useState: (initial) => {
        const index = stateIndex++;
        return [["", "", "idle", true][index] ?? initial, () => {}];
      },
    },
    useT: () => tForTest,
  };
  vm.runInNewContext(onboardingPairingCardSourceForTest(), context);

  const rendered = context.globalThis.__testExports.OnboardingPairingCard({
    onboarding: {
      extensionName: "slack",
      strategy: "oauth",
      submitLabel: "Connect Slack",
      submittingLabel: "Connecting...",
    },
    onConfigure: async () => {},
  });

  const button = findComponent(rendered, context.Button);
  const props = componentProps(button, context.Button);
  assert.equal(props.loading, true);
  assert.ok(JSON.stringify(rendered).includes("Connecting..."));
});

test("OnboardingPairingCard keeps OAuth configuration loading after the popup opens", async () => {
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
        return [
          ["", "", "idle", false][index] ?? initial,
          (value) => stateUpdates.push({ index, value }),
        ];
      },
    },
    useT: () => tForTest,
  };
  vm.runInNewContext(onboardingPairingCardSourceForTest(), context);

  const rendered = context.globalThis.__testExports.OnboardingPairingCard({
    onboarding: { extensionName: "slack", strategy: "oauth" },
    onConfigure: async () => ({ flow_id: "flow-slack" }),
  });

  const button = findComponent(rendered, context.Button);
  const props = componentProps(button, context.Button);
  await props.onClick();

  assert.deepEqual(
    stateUpdates.filter((update) => update.index === 3).map((update) => update.value),
    [true],
  );
});

test("OnboardingPairingCard exits the connect spinner and surfaces the error when the OAuth flow fails", () => {
  const stateUpdates = [];
  let stateIndex = 0;
  const context = {
    Button() {},
    channelConnectionDisplayName,
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    React: {
      // code="", error="", status="idle", isConfiguring=true (spinner held by
      // an in-flight OAuth flow whose popup has since failed/expired).
      useState: (initial) => {
        const index = stateIndex++;
        return [
          ["", "", "idle", true][index] ?? initial,
          (value) => stateUpdates.push({ index, value }),
        ];
      },
    },
    useT: () => tForTest,
  };
  vm.runInNewContext(onboardingPairingCardSourceForTest(), context);

  context.globalThis.__testExports.OnboardingPairingCard({
    onboarding: {
      extensionName: "slack",
      strategy: "oauth",
      oauthError: "Authorization failed. Try connecting again.",
    },
    onConfigure: async () => {},
  });

  assert.ok(
    stateUpdates.some((update) => update.index === 3 && update.value === false),
    "a failed OAuth flow must exit the connect spinner",
  );
  assert.ok(
    stateUpdates.some(
      (update) =>
        update.index === 1 && update.value === "Authorization failed. Try connecting again.",
    ),
    "a failed OAuth flow must surface the retryable error copy",
  );
});
