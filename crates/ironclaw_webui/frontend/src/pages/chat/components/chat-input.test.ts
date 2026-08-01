// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";
import vm from "node:vm";
import { componentSourceForTest } from "../../../lib/vm-component-harness";

function chatInputSourceForTest() {
  return componentSourceForTest(
    new URL("./chat-input.tsx", import.meta.url),
    "ChatInput",
  );
}

function renderChatInput({ sendDisabled = true, statusText = "" } = {}) {
  const context = {
    globalThis: {},
    Icon() {},
    Button() {},
    ModeSelector() {},
    React: {
      useCallback: (callback) => callback,
      useEffect: () => {},
      useRef: (initial) => ({ current: initial }),
      useState: (initial) => [
        typeof initial === "function" ? initial() : initial,
        () => {},
      ],
    },
    authScope: () => "test-scope",
    commandMenuMatches: () => [],
    commandMenuToken: () => null,
    getDraft: () => "",
    getStagedAttachments: () => [],
    INITIAL_COMMAND_MENU_SELECTION: { index: 0, dismissed: false },
    NEW_DRAFT_KEY: "new",
    useAttachmentConfig: () => ({ accept: [] }),
    useAgentMode: () => ["suggest", () => {}],
    useT: () => (key) => key,
  };

  vm.runInNewContext(chatInputSourceForTest(), context);
  return context.globalThis.__testExports.ChatInput({
    onSend: () => {},
    disabled: false,
    sendDisabled,
    draftKey: "draft-key",
    statusText,
  });
}

function containsScalar(value, expected, seen = new Set()) {
  if (value === expected) return true;
  if (!value || typeof value !== "object" || seen.has(value)) return false;
  seen.add(value);
  return Object.values(value).some((child) =>
    Array.isArray(child)
      ? child.some((item) => containsScalar(item, expected, seen))
      : containsScalar(child, expected, seen)
  );
}

test("ChatInput only shows explicit composer status text while sending is disabled", () => {
  const busyComposer = renderChatInput();
  assert.equal(
    containsScalar(busyComposer, "chat.statusWorking"),
    false,
    "run progress belongs beside the thread indicator, not in the composer",
  );

  const gatedComposer = renderChatInput({ statusText: "Approval required" });
  assert.equal(
    containsScalar(gatedComposer, "Approval required"),
    true,
    "approval and cooldown notices should remain visible in the composer",
  );
});
