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
  return `${lines.join("\n")}\nglobalThis.__testExports = { isSlackChannelEnabled, slackBuiltinStatus, isSlackAdminManagedAction, isSlackInboundProofCodeAction, findSlackConnectAction };`;
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

test("findSlackConnectAction prefers admin channel management over personal pairing", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(channelsTabSourceForTest(), context);
  const { findSlackConnectAction } = context.globalThis.__testExports;
  const personal = { channel: "slack", strategy: "inbound_proof_code" };
  const admin = { channel: "slack", strategy: "admin_managed_channels" };

  assert.equal(findSlackConnectAction([personal]), personal);
  assert.equal(findSlackConnectAction([personal, admin]), admin);
});
