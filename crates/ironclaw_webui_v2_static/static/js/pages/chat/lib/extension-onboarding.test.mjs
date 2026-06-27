import assert from "node:assert/strict";
import test from "node:test";

import {
  onboardingFromExtensionActivatePreview,
  onboardingFromToolMessages,
} from "./extension-onboarding.js";

const slackActivationOutput = {
  message:
    "Slack is installed as an inbound channel, but the user's Slack account still needs pairing. Tell the user to DM the Slack app; the bot will reply with a pairing code. The user should paste that code into the Slack account connection panel in WebChat, not into normal chat.",
  package_ref: { id: "slack", kind: "extension" },
  payload: {
    activated: true,
    kind: "extension_activate",
    visible_capability_ids: [],
  },
  phase: "active",
};

test("onboardingFromExtensionActivatePreview: opens Slack pairing panel from activation preview", () => {
  const onboarding = onboardingFromExtensionActivatePreview(
    {
      capability_id: "builtin.extension_activate",
      thread_id: "thread-1",
      output_preview: JSON.stringify(slackActivationOutput),
    },
    "thread-1",
  );

  assert.equal(onboarding?.state, "pairing_required");
  assert.equal(onboarding?.extensionName, "slack");
  assert.equal(onboarding?.threadId, "thread-1");
  assert.equal(onboarding?.inputPlaceholder, "Enter Slack pairing code");
});

test("onboardingFromToolMessages: opens panel from reloaded timeline tool card", () => {
  const onboarding = onboardingFromToolMessages(
    [
      {
        id: "tool-extension-activate",
        role: "tool_activity",
        capabilityId: "builtin.extension_activate",
        toolStatus: "success",
        toolResultPreview: JSON.stringify(slackActivationOutput),
      },
    ],
    "thread-1",
  );

  assert.equal(onboarding?.state, "pairing_required");
  assert.equal(onboarding?.extensionName, "slack");
  assert.match(onboarding?.instructions, /Paste the code here/);
});

test("onboardingFromToolMessages: suppresses stale Slack panel after continuation", () => {
  const onboarding = onboardingFromToolMessages(
    [
      {
        id: "tool-extension-activate",
        role: "tool_activity",
        capabilityId: "builtin.extension_activate",
        toolStatus: "success",
        toolResultPreview: JSON.stringify(slackActivationOutput),
      },
      {
        id: "msg-continuation",
        role: "user",
        content: "Slack is connected. Continue the previous request.",
      },
    ],
    "thread-1",
  );

  assert.equal(onboarding, null);
});

test("onboardingFromToolMessages: tags the source tool id and suppresses dismissed activations", () => {
  const messages = [
    {
      id: "tool-extension-activate",
      role: "tool_activity",
      capabilityId: "builtin.extension_activate",
      toolStatus: "success",
      toolResultPreview: JSON.stringify(slackActivationOutput),
    },
  ];

  const open = onboardingFromToolMessages(messages, "thread-1", new Set());
  assert.equal(open?.state, "pairing_required");
  assert.equal(
    open?.sourceMessageId,
    "tool-extension-activate",
    "onboarding must carry the source tool-message id so a dismissal can be recorded",
  );

  const suppressed = onboardingFromToolMessages(
    messages,
    "thread-1",
    new Set(["tool-extension-activate"]),
  );
  assert.equal(
    suppressed,
    null,
    "a dismissed activation must not re-open the pairing panel on the next render",
  );
});

test("onboardingFromExtensionActivatePreview: tags the source tool id and suppresses dismissed activations", () => {
  const preview = {
    capability_id: "builtin.extension_activate",
    invocation_id: "inv-1",
    thread_id: "thread-1",
    output_preview: JSON.stringify(slackActivationOutput),
  };

  const open = onboardingFromExtensionActivatePreview(preview, "thread-1", new Set());
  assert.equal(open?.state, "pairing_required");
  assert.equal(
    open?.sourceMessageId,
    "tool-inv-1",
    "preview onboarding id must match the reloaded tool-message id (tool-<invocation_id>)",
  );

  const suppressed = onboardingFromExtensionActivatePreview(
    preview,
    "thread-1",
    new Set(["tool-inv-1"]),
  );
  assert.equal(suppressed, null);
});
