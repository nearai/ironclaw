import assert from "node:assert/strict";
import test from "node:test";

import { resolveChannelConnectCommand } from "./channel-connect.js";

const slack = {
  channel: "slack",
  display_name: "Slack",
  command_aliases: ["slack", "slack account"],
};

const telegram = {
  channel: "telegram",
  display_name: "Telegram",
  command_aliases: ["telegram"],
};

test("resolveChannelConnectCommand detects explicit Slack connect requests", () => {
  assert.equal(resolveChannelConnectCommand("connect my Slack account", [slack]), slack);
  assert.equal(resolveChannelConnectCommand("pair slack", [slack]), slack);
  assert.equal(resolveChannelConnectCommand("link the slack app", [slack]), slack);
});

test("resolveChannelConnectCommand leaves admin-managed Slack channel commands out of chat", () => {
  const personalSlack = {
    channel: "slack",
    display_name: "Slack",
    strategy: "inbound_proof_code",
    command_aliases: ["slack", "slack account", "slack pairing"],
  };
  const adminSlack = {
    channel: "slack",
    display_name: "Slack",
    strategy: "admin_managed_channels",
    command_aliases: [],
  };

  assert.equal(
    resolveChannelConnectCommand("connect slack", [personalSlack, adminSlack]),
    personalSlack,
  );
  assert.equal(
    resolveChannelConnectCommand("connect slack channel", [personalSlack, adminSlack]),
    null,
  );
  assert.equal(
    resolveChannelConnectCommand("connect slack allowlist", [
      personalSlack,
      { ...adminSlack, command_aliases: ["slack allowlist"] },
    ]),
    null,
  );
});

test("resolveChannelConnectCommand only suppresses Slack channel management wording", () => {
  assert.equal(resolveChannelConnectCommand("connect telegram channel", [telegram]), telegram);
});

test("resolveChannelConnectCommand leaves ordinary Slack prompts for the model", () => {
  assert.equal(resolveChannelConnectCommand("send a message to Slack", [slack]), null);
  assert.equal(resolveChannelConnectCommand("what is slack?", [slack]), null);
  assert.equal(resolveChannelConnectCommand("connect the two ideas", [slack]), null);
});
