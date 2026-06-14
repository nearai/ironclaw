import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function channelConnectCardSourceForTest() {
  const source = readFileSync(new URL("./channel-connect-card.js", import.meta.url), "utf8");
  const lines = [];
  for (const line of source.split("\n")) {
    if (line.startsWith("import ")) continue;
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { isSlackStrategy, isConnectedChannel, shouldRenderSlackPairingSection, ChannelConnectCard };`;
}

function renderHtml(strings, ...values) {
  return strings.reduce((rendered, part, index) => {
    const value = index < values.length ? values[index] : "";
    return `${rendered}${part}${value ?? ""}`;
  }, "");
}

test("isSlackStrategy gates the Slack personal pairing renderer", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(channelConnectCardSourceForTest(), context);
  const { isSlackStrategy } = context.globalThis.__testExports;

  assert.equal(
    isSlackStrategy(
      { channel: "slack", strategy: "inbound_proof_code" },
      "inbound_proof_code",
    ),
    true,
  );
  assert.equal(
    isSlackStrategy(
      { channel: "slack", strategy: "inbound_proof_code" },
      "admin_managed_channels",
    ),
    false,
  );
  assert.equal(
    isSlackStrategy(
      { channel: "teams", strategy: "inbound_proof_code" },
      "inbound_proof_code",
    ),
    false,
  );
});

test("connected Slack does not render the personal pairing section", () => {
  const context = { globalThis: {} };
  vm.runInNewContext(channelConnectCardSourceForTest(), context);
  const { isConnectedChannel, shouldRenderSlackPairingSection } =
    context.globalThis.__testExports;

  assert.equal(
    isConnectedChannel({
      channel: "slack",
      strategy: "inbound_proof_code",
      connection_status: "connected",
    }),
    true,
  );
  assert.equal(
    shouldRenderSlackPairingSection({
      channel: "slack",
      strategy: "inbound_proof_code",
      connection_status: "connected",
    }),
    false,
  );
  assert.equal(
    shouldRenderSlackPairingSection({
      channel: "slack",
      strategy: "inbound_proof_code",
      connection_status: "disconnected",
    }),
    true,
  );
});

test("connected Slack card renders connected state without pairing section", () => {
  const context = {
    globalThis: {},
    html: renderHtml,
    Icon: "Icon",
    SlackPairingSection: "SlackPairingSection",
  };
  vm.runInNewContext(channelConnectCardSourceForTest(), context);
  const { ChannelConnectCard } = context.globalThis.__testExports;

  const rendered = ChannelConnectCard({
    connectAction: {
      channel: "slack",
      display_name: "Slack",
      strategy: "inbound_proof_code",
      connection_status: "connected",
      action: { instructions: "Message the Slack app, then enter the code here." },
    },
  });

  assert.match(rendered, /Connected Slack/);
  assert.match(rendered, /Slack is connected\./);
  assert.doesNotMatch(rendered, /SlackPairingSection/);
  assert.doesNotMatch(rendered, /Message the Slack app/);
});
