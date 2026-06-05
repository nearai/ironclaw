import assert from "node:assert/strict";
import test from "node:test";

import { resolveChannelConnectCommand } from "./channel-connect.js";

const slack = {
  channel: "slack",
  display_name: "Slack",
  command_aliases: ["slack", "slack account"],
};

test("resolveChannelConnectCommand detects explicit Slack connect requests", () => {
  assert.equal(resolveChannelConnectCommand("connect my Slack account", [slack]), slack);
  assert.equal(resolveChannelConnectCommand("pair slack", [slack]), slack);
  assert.equal(resolveChannelConnectCommand("link the slack app", [slack]), slack);
});

test("resolveChannelConnectCommand leaves ordinary Slack prompts for the model", () => {
  assert.equal(resolveChannelConnectCommand("send a message to Slack", [slack]), null);
  assert.equal(resolveChannelConnectCommand("what is slack?", [slack]), null);
  assert.equal(resolveChannelConnectCommand("connect the two ideas", [slack]), null);
});
