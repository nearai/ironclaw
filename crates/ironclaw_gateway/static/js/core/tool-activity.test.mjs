import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

import {
  chatChildKinds,
  createChatContainer,
  createMessageElement,
  FakeElement,
} from "./test-dom.mjs";

function createToolActivityHarness(initialChildren = []) {
  const container = createChatContainer(initialChildren);
  const context = {
    clearInterval: () => {},
    Date,
    document: {
      createElement: (tagName) => new FakeElement(tagName),
      getElementById: (id) => (id === "chat-messages" ? container : null),
    },
    isFinite,
    Map,
    Number,
    setInterval: () => 1,
  };
  vm.runInNewContext(
    readFileSync(new URL("./tool-activity.js", import.meta.url), "utf8"),
    context,
  );

  return {
    container,
    controller: context.createToolActivityController({
      containerId: "chat-messages",
    }),
  };
}

test("tool activity started after a trailing assistant reply renders before that reply", () => {
  const { container, controller } = createToolActivityHarness([
    createMessageElement("user"),
    createMessageElement("assistant"),
  ]);

  controller.startTool({
    call_id: "call-extension-search",
    name: "extension_search",
  });

  assert.deepEqual(chatChildKinds(container), ["user", "activity", "assistant"]);

  controller.completeTool({
    call_id: "call-extension-search",
    name: "extension_search",
    success: true,
    duration_ms: 25,
  });
  controller.finalizeGroup();

  assert.deepEqual(chatChildKinds(container), ["user", "activity", "assistant"]);
});

test("tool activity after a follow-up user message stays with the active follow-up turn", () => {
  const { container, controller } = createToolActivityHarness([
    createMessageElement("user"),
    createMessageElement("assistant"),
    createMessageElement("user"),
  ]);

  controller.startTool({
    call_id: "call-calendar",
    name: "calendar",
  });

  assert.deepEqual(chatChildKinds(container), [
    "user",
    "assistant",
    "user",
    "activity",
  ]);
});

test("tool activity does not skip non-message cards after an assistant reply", () => {
  const card = new FakeElement("div");
  card.className = "auth-card";
  const { container, controller } = createToolActivityHarness([
    createMessageElement("user"),
    createMessageElement("assistant"),
    card,
  ]);

  controller.startTool({
    call_id: "call-drive",
    name: "drive",
  });

  assert.deepEqual(chatChildKinds(container), [
    "user",
    "assistant",
    "div",
    "activity",
  ]);
});

test("tool activity appends normally when there is no existing chat message", () => {
  const { container, controller } = createToolActivityHarness();

  controller.startTool({
    call_id: "call-empty",
    name: "bootstrap",
  });

  assert.deepEqual(chatChildKinds(container), ["activity"]);
});
