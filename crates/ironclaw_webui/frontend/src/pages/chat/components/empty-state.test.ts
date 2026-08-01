// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";
import vm from "node:vm";
import {
  componentProps,
  componentSourceForTest,
  findComponent,
} from "../../../lib/vm-component-harness";

// Same vm-harness convention as chat-input.test.ts (shared plumbing lives in
// lib/vm-component-harness.ts): read the real source, strip imports, rename
// the export, and stub the module's free variables (including its child
// components) so the vm can evaluate it directly. This is the only way to
// observe what EmptyState actually forwards to its internal ChatInput —
// chat.test.ts stubs both components as no-ops and so cannot see prop
// plumbing between them.
function emptyStateSourceForTest() {
  return componentSourceForTest(
    new URL("./empty-state.tsx", import.meta.url),
    "EmptyState",
  );
}

function renderEmptyState(props = {}) {
  const components = {
    Icon() {},
    ChatInput() {},
    AutomationCarousel() {},
  };
  const context = {
    ...components,
    globalThis: {},
    React: {
      useState: (initial) => [initial, () => {}],
    },
    useAutomationTasks: () => ({ loading: false, tasks: [] }),
    useT: () => (key) => key,
  };
  vm.runInNewContext(emptyStateSourceForTest(), context);
  const tree = context.globalThis.__testExports.EmptyState({
    onSuggestion: () => {},
    onSend: () => {},
    disabled: false,
    sendDisabled: false,
    initialText: "",
    resetKey: "",
    draftKey: "draft-key",
    context: {},
    statusText: "",
    canCancel: false,
    onCancel: () => {},
    ...props,
  });
  return { tree, components };
}

test("EmptyState forwards a non-empty commands list to its composer so the menu is reachable from the landing view", () => {
  const commands = [
    { name: "status", title: "Status", description: "d", usage: "/status" },
  ];
  const { tree, components } = renderEmptyState({ commands });

  const chatInput = findComponent(tree, components.ChatInput);
  const props = componentProps(chatInput, components.ChatInput);
  assert.deepEqual(props.commands, commands);
});
