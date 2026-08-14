// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";
import vm from "node:vm";

import { componentProps, findComponent } from "../../../lib/vm-component-harness";
import { sourceForVmTest } from "../../../test-support/vm-module-harness";

function renderCard({ gate, onCancel = () => {} }) {
  const context = {
    AuthGateShell() {},
    Button: "button",
    DeviceLinkPanel() {},
    globalThis: {},
    React: {},
    useT: () => (key, params = {}) =>
      Object.entries(params).reduce(
        (text, [name, value]) => text.replace(`{${name}}`, String(value)),
        key,
      ),
  };
  vm.runInNewContext(
    sourceForVmTest("./auth-device-link-card.tsx", ["AuthDeviceLinkCard"], import.meta.url),
    context,
  );
  const rendered = context.globalThis.__testExports.AuthDeviceLinkCard({ gate, onCancel });
  return { rendered, context };
}

function propsFor(rendered, component) {
  const node = findComponent(rendered, component);
  assert.ok(node, "expected the component to render");
  return componentProps(node, component);
}

const GATE = {
  kind: "auth_required",
  challengeKind: "device_link",
  runId: "run-1",
  gateRef: "gate-1",
  invocationId: "invocation-1",
  provider: "telegram",
  accountLabel: "Personal account",
  headline: "Link your Telegram account",
  body: "Linking lets the agent read and send as you.",
  threadId: "thread-1",
  deviceLink: {
    provider: "telegram",
    displayName: "Telegram",
    step: "display",
    instructions: "Open Telegram and scan this.",
    qrPayload: "tg://login?token=AAAA",
    revision: 3,
    terminal: false,
  },
};

test("AuthDeviceLinkCard hands the gate's scope and initial frame to the shared flow panel", () => {
  const { rendered, context } = renderCard({ gate: GATE });

  const panel = propsFor(rendered, context.DeviceLinkPanel);
  assert.equal(panel.provider, "telegram");
  assert.equal(panel.displayName, "Telegram");
  assert.equal(panel.initialFrame, GATE.deviceLink);
  assert.equal(panel.runId, "run-1");
  assert.equal(panel.gateRef, "gate-1");
  assert.equal(panel.invocationId, "invocation-1");
  assert.equal(panel.threadId, "thread-1");

  const shell = propsFor(rendered, context.AuthGateShell);
  assert.equal(shell.challengeKind, "device_link");
  assert.equal(shell.headline, "Link your Telegram account");
  assert.equal(shell.accountLabel, "Personal account");
  assert.equal(shell.body, "Linking lets the agent read and send as you.");
  // The whole point of the shell is that the drawer, not the card, owns chrome.
  assert.equal(shell.testId, "auth-device-link-card");
});

test("AuthDeviceLinkCard renders without a frame and still names the account", () => {
  // A projection row written before the device-link field existed carries no
  // frame; the card must render and let the panel start its own flow.
  const { rendered, context } = renderCard({
    gate: { ...GATE, deviceLink: null, headline: "" },
  });

  const panel = propsFor(rendered, context.DeviceLinkPanel);
  assert.equal(panel.initialFrame, null);
  assert.equal(panel.displayName, "Personal account");

  const shell = propsFor(rendered, context.AuthGateShell);
  assert.equal(shell.headline, "deviceLink.title");
});

test("AuthDeviceLinkCard cancel abandons the gate", async () => {
  const cancels = [];
  const { rendered } = renderCard({ gate: GATE, onCancel: () => cancels.push("cancel") });

  const onClicks = [];
  const walk = (value) => {
    if (Array.isArray(value)) {
      value.forEach(walk);
      return;
    }
    if (!value || !Array.isArray(value.strings) || !Array.isArray(value.values)) return;
    value.strings.forEach((part, index) => {
      if (part.endsWith("onClick=")) onClicks.push(value.values[index]);
    });
    value.values.forEach(walk);
  };
  walk(rendered);

  assert.equal(onClicks.length, 1, "the card owns exactly one action of its own");
  await onClicks[0]();
  assert.deepEqual(cancels, ["cancel"]);
});
