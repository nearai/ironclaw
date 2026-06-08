import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function channelsTabSourceForTest() {
  const source = readFileSync(new URL("./channels-tab.js", import.meta.url), "utf8");
  const lines = [];
  for (const line of source.split("\n")) {
    if (line.startsWith("import ")) continue;
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { SlackBuiltInConnectAction, isSlackChannelEnabled, slackBuiltinStatus, isSlackAdminManagedAction, isSlackInboundProofCodeAction, findSlackConnectAction, findSlackConnectActions };`;
}

function slackBuiltInConnectActionForTest(slackConnectAction, slackConnectActions) {
  const context = {
    globalThis: {},
    SlackChannelPicker() {},
    SlackPairingSection() {},
    html(strings, ...values) {
      return { strings: Array.from(strings), values };
    },
  };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  return {
    rendered: context.globalThis.__testExports.SlackBuiltInConnectAction({
      slackConnectAction,
      slackConnectActions,
    }),
    SlackChannelPicker: context.SlackChannelPicker,
    SlackPairingSection: context.SlackPairingSection,
  };
}

test("isSlackChannelEnabled covers all Slack channel ids", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  const { isSlackChannelEnabled } = context.globalThis.__testExports;

  assert.equal(isSlackChannelEnabled(["slack"]), true);
  assert.equal(isSlackChannelEnabled(["slack_v2"]), true);
  assert.equal(isSlackChannelEnabled(["slack-v2"]), true);
  assert.equal(isSlackChannelEnabled([]), false);
  assert.equal(isSlackChannelEnabled(["other"]), false);
});

test("slackBuiltinStatus labels the Reborn admin-managed channel flow", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  const { slackBuiltinStatus } = context.globalThis.__testExports;

  assert.equal(JSON.stringify(slackBuiltinStatus(true, null)), JSON.stringify({
    label: "on",
    tone: "success",
  }));
  assert.equal(
    JSON.stringify(slackBuiltinStatus(false, { strategy: "admin_managed_channels" })),
    JSON.stringify({ label: "manage", tone: "info" }),
  );
  assert.equal(
    JSON.stringify(slackBuiltinStatus(false, { strategy: "inbound_proof_code" })),
    JSON.stringify({ label: "connect", tone: "info" }),
  );
  assert.equal(JSON.stringify(slackBuiltinStatus(false, null)), JSON.stringify({
    label: "off",
    tone: "muted",
  }));
});

test("Slack built-in action predicates keep admin picker and proof-code pairing distinct", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  const { isSlackAdminManagedAction, isSlackInboundProofCodeAction } =
    context.globalThis.__testExports;

  assert.equal(
    isSlackAdminManagedAction({ channel: "slack", strategy: "admin_managed_channels" }),
    true,
  );
  assert.equal(
    isSlackInboundProofCodeAction({ channel: "slack", strategy: "inbound_proof_code" }),
    true,
  );
  assert.equal(
    isSlackAdminManagedAction({ channel: "slack", strategy: "inbound_proof_code" }),
    false,
  );
  assert.equal(
    isSlackInboundProofCodeAction({ channel: "teams", strategy: "inbound_proof_code" }),
    false,
  );
});

test("findSlackConnectActions keeps admin channel management and personal pairing", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  const { findSlackConnectAction, findSlackConnectActions } =
    context.globalThis.__testExports;
  const personal = { channel: "slack", strategy: "inbound_proof_code" };
  const admin = { channel: "slack", strategy: "admin_managed_channels" };

  assert.equal(findSlackConnectAction([personal]), personal);
  assert.equal(findSlackConnectAction([personal, admin]), admin);
  const actions = findSlackConnectActions([personal, admin]);
  assert.equal(actions.length, 2);
  assert.equal(actions[0].strategy, "admin_managed_channels");
  assert.equal(actions[1].strategy, "inbound_proof_code");
});

test("SlackBuiltInConnectAction renders every supported Slack action", () => {
  const personal = { channel: "slack", strategy: "inbound_proof_code", action: {} };
  const admin = { channel: "slack", strategy: "admin_managed_channels", action: {} };

  const adminView = slackBuiltInConnectActionForTest(admin);
  assert.equal(adminView.rendered.values[0][0].values[0], adminView.SlackChannelPicker);

  const personalView = slackBuiltInConnectActionForTest(personal);
  assert.equal(personalView.rendered.values[0][0].values[0], personalView.SlackPairingSection);

  const combinedView = slackBuiltInConnectActionForTest(null, [admin, personal]);
  assert.equal(combinedView.rendered.values[0][0].values[0], combinedView.SlackChannelPicker);
  assert.equal(combinedView.rendered.values[0][1].values[0], combinedView.SlackPairingSection);

  const unhandledView = slackBuiltInConnectActionForTest({
    channel: "slack",
    strategy: "admin_managed_unknown",
    action: {},
  });
  assert.equal(unhandledView.rendered, null);
});
