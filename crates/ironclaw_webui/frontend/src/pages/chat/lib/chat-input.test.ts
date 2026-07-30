// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

import {
  INITIAL_COMMAND_MENU_SELECTION,
  commandMenuMatches,
  commandMenuSelectionReducer,
  commandMenuToken,
} from "./chat-commands";

// Wire shape from `GET /api/webchat/v2/commands`. Two commands share the
// "mo" prefix so ArrowDown/wraparound has somewhere to move.
const MENU_COMMANDS = [
  {
    name: "model",
    title: "Model",
    description: "Show or switch the active LLM provider and model",
    usage: "/model [provider] [name]",
  },
  {
    name: "modelinfo",
    title: "Model info",
    description: "Show detailed information about the active model",
    usage: "/modelinfo --verbose",
  },
  {
    name: "status",
    title: "Status",
    description: "Show what the assistant is doing",
    usage: "/status",
  },
];

function chatInputSourceForTest() {
  const source = readFileSync(
    new URL("../components/chat-input.tsx", import.meta.url),
    "utf8",
  );
  const lines = [];
  let skippingImport = false;
  for (const line of source.split("\n")) {
    if (!skippingImport && line.startsWith("import ")) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    if (skippingImport) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    lines.push(line.replace("export function ChatInput", "function ChatInput"));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { ChatInput };`;
}

function findComponent(node, component) {
  if (!node || typeof node !== "object") return null;
  if (!Array.isArray(node.values)) return null;
  const componentIndex = node.values.indexOf(component);
  if (componentIndex >= 0) {
    return node;
  }
  for (const value of node.values) {
    const found = findComponent(value, component);
    if (found) return found;
  }
  return null;
}

// HTML attribute names may contain hyphens (for example, data-testid).
const HTML_ATTRIBUTE_PATTERN = /([A-Za-z][A-Za-z0-9-]*)=\s*$/;

function componentProps(node, component) {
  const props = {};
  const start = node.values.indexOf(component);
  for (let index = start + 1; index < node.values.length; index += 1) {
    const name = node.strings[index]?.match(HTML_ATTRIBUTE_PATTERN)?.[1];
    if (name) props[name] = node.values[index];
  }
  return props;
}

function templateProps(node) {
  const props = {};
  for (let index = 0; index < node.values.length; index += 1) {
    const name = node.strings[index]?.match(HTML_ATTRIBUTE_PATTERN)?.[1];
    if (name) props[name] = node.values[index];
  }
  return props;
}

function findNode(node, predicate) {
  if (!node || typeof node !== "object") return null;
  if (Array.isArray(node.strings) && predicate(node)) return node;
  if (!Array.isArray(node.values)) return null;
  for (const value of node.values) {
    const found = findNode(value, predicate);
    if (found) return found;
  }
  return null;
}

// Flattens a synthetic jsx node's `children` into the plain text it would
// render, so assertions can check for title/description/usage substrings
// without hand-walking the tree.
function extractText(node) {
  if (node == null) return "";
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node.children)) return node.children.map(extractText).join("");
  return "";
}

async function flushAsyncHandlers() {
  await new Promise((resolve) => setImmediate(resolve));
}

function renderChatInput({
  onSend = async () => {},
  onCancel,
  setCalls = [],
  refs = [],
  disabled = true,
  sendDisabled,
  canCancel = true,
  draft = "",
  draftKey,
  authScopeFn = () => "test-scope",
  setDraftCalls = [],
  commands = [],
} = {}) {
  const components = {
    Button() {},
    Icon() {},
  };
  let stateIndex = 0;
  const context = {
    ...components,
    React: {
      useCallback: (fn) => fn,
      useEffect: () => {},
      useRef: (initial = null) => {
        const ref = { current: initial };
        refs.push(ref);
        return ref;
      },
      useState: (initial) => {
        const index = stateIndex++;
        let value = typeof initial === "function" ? initial() : initial;
        return [
          value,
          (next) => {
            value = typeof next === "function" ? next(value) : next;
            setCalls.push({ index, value });
          },
        ];
      },
    },
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    useT: () => (key) => key,
    authScope: authScopeFn,
    stageFiles: async () => ({ staged: [], errors: [] }),
    commandMenuMatches,
    commandMenuToken,
    commandMenuSelectionReducer,
    INITIAL_COMMAND_MENU_SELECTION,
    useAttachmentConfig: () => ({
      accept: [],
      maxCount: 10,
      maxFileBytes: 1024,
      maxTotalBytes: 2048,
    }),
    NEW_DRAFT_KEY: "__new__",
    clearDraft: () => {},
    clearStagedAttachments: () => {},
    getDraft: () => draft,
    getStagedAttachments: () => [],
    setDraft: (key, text) => setDraftCalls.push({ key, text }),
    setStagedAttachments: () => {},
    window: {
      clearTimeout: () => {},
      requestAnimationFrame: (fn) => fn(),
      setTimeout: () => 1,
    },
  };

  vm.runInNewContext(chatInputSourceForTest(), context);
  const tree = context.globalThis.__testExports.ChatInput({
    onSend,
    onCancel,
    disabled,
    sendDisabled,
    canCancel,
    draftKey,
    commands,
  });
  return { tree, components };
}

test("ChatInput cancel button invokes onCancel and resets cancelling state", async () => {
  const setCalls = [];
  let cancelCalls = 0;
  let resolveCancel;
  const { tree, components } = renderChatInput({
    setCalls,
    onCancel: async () =>
      new Promise((resolve) => {
        cancelCalls += 1;
        resolveCancel = resolve;
      }),
  });

  const cancelButton = findComponent(tree, components.Button);
  const props = componentProps(cancelButton, components.Button);
  assert.equal(props["data-testid"], "chat-cancel-run");
  const cancelPromise = props.onClick();

  assert.equal(cancelCalls, 1);
  assert.deepEqual(setCalls.slice(0, 1), [{ index: 4, value: true }]);

  resolveCancel();
  await cancelPromise;

  assert.deepEqual(setCalls.slice(-1), [{ index: 4, value: false }]);
});

test("ChatInput cancel button resets cancelling state after rejection", async () => {
  const setCalls = [];
  const { tree, components } = renderChatInput({
    setCalls,
    onCancel: async () => {
      throw new Error("cancel failed");
    },
  });

  const cancelButton = findComponent(tree, components.Button);
  const props = componentProps(cancelButton, components.Button);
  await assert.rejects(props.onClick(), /cancel failed/);

  assert.deepEqual(setCalls, [
    { index: 4, value: true },
    { index: 4, value: false },
  ]);
});

test("ChatInput keeps the textarea editable when only submit is disabled", () => {
  const { tree, components } = renderChatInput({
    disabled: false,
    sendDisabled: true,
    canCancel: false,
    draft: "next thought",
  });

  const textarea = findNode(tree, (node) =>
    node.strings.some((part) => part.includes("<textarea")),
  );
  const textareaProps = templateProps(textarea);
  assert.equal(textareaProps.disabled, false);
  assert.equal(textareaProps.value, "next thought");

  const sendButton = findComponent(tree, components.Button);
  const sendProps = componentProps(sendButton, components.Button);
  assert.equal(sendProps.disabled, true);
});

test("ChatInput blocks Enter send when only submit is disabled", async () => {
  let sendCalls = 0;
  const { tree } = renderChatInput({
    disabled: false,
    sendDisabled: true,
    canCancel: false,
    draft: "draft while busy",
    onSend: async () => {
      sendCalls += 1;
    },
  });

  const textarea = findNode(tree, (node) =>
    node.strings.some((part) => part.includes("<textarea")),
  );
  const textareaProps = templateProps(textarea);
  let prevented = false;
  textareaProps.onKeyDown({
    key: "Enter",
    shiftKey: false,
    preventDefault: () => {
      prevented = true;
    },
  });
  await Promise.resolve();

  assert.equal(prevented, true);
  assert.equal(sendCalls, 0);
});

test("ChatInput blocks Enter send from current DOM disabled state", async () => {
  let sendCalls = 0;
  const { tree } = renderChatInput({
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "draft while busy",
    onSend: async () => {
      sendCalls += 1;
    },
  });

  const textarea = findNode(tree, (node) =>
    node.strings.some((part) => part.includes("<textarea")),
  );
  const textareaProps = templateProps(textarea);
  let prevented = false;
  textareaProps.onKeyDown({
    key: "Enter",
    shiftKey: false,
    currentTarget: { dataset: { sendDisabled: "true" } },
    preventDefault: () => {
      prevented = true;
    },
  });
  await Promise.resolve();

  assert.equal(prevented, true);
  assert.equal(sendCalls, 0);
});

test("ChatInput sends the latest text when Enter follows input before rerender", async () => {
  const sentContents = [];
  const { tree } = renderChatInput({
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "",
    onSend: async (content) => {
      sentContents.push(content);
    },
  });

  const textarea = findNode(tree, (node) =>
    node.strings.some((part) => part.includes("<textarea")),
  );
  const textareaProps = templateProps(textarea);

  // Browser input updates the live value before React commits the next render.
  // Enter in that window must submit the live value, not the stale render state.
  textareaProps.onChange({ currentTarget: { value: "follow-up right away" } });
  textareaProps.onKeyDown({
    key: "Enter",
    shiftKey: false,
    preventDefault: () => {},
  });
  await flushAsyncHandlers();

  assert.deepEqual(sentContents, ["follow-up right away"]);
});

test("ChatInput preserves draft when caller refuses send", async () => {
  const setCalls = [];
  let sendCalls = 0;
  const { tree } = renderChatInput({
    setCalls,
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "draft while busy",
    onSend: async () => {
      sendCalls += 1;
      return null;
    },
  });

  const textarea = findNode(tree, (node) =>
    node.strings.some((part) => part.includes("<textarea")),
  );
  const textareaProps = templateProps(textarea);
  textareaProps.onKeyDown({
    key: "Enter",
    shiftKey: false,
    preventDefault: () => {},
  });
  await flushAsyncHandlers();
  textareaProps.onKeyDown({
    key: "Enter",
    shiftKey: false,
    preventDefault: () => {},
  });
  await flushAsyncHandlers();

  assert.equal(sendCalls, 2);
  assert.deepEqual(
    setCalls
      .filter((call) => call.index === 0)
      .map((call) => call.value),
    ["", "draft while busy", "", "draft while busy"],
  );
});

test("ChatInput clears the textarea as soon as send starts", async () => {
  const setCalls = [];
  let sendCalls = 0;
  const { tree } = renderChatInput({
    setCalls,
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "ship it now",
    onSend: async () =>
      new Promise(() => {
        sendCalls += 1;
      }),
  });

  const textarea = findNode(tree, (node) =>
    node.strings.some((part) => part.includes("<textarea")),
  );
  const textareaProps = templateProps(textarea);
  textareaProps.onKeyDown({
    key: "Enter",
    shiftKey: false,
    preventDefault: () => {},
  });
  await Promise.resolve();

  assert.equal(sendCalls, 1);
  assert.equal(setCalls[0].index, 3);
  assert.equal(setCalls[0].value, true);
  assert.equal(setCalls[1].index, 0);
  assert.equal(setCalls[1].value, "");
  assert.equal(setCalls[2].index, 1);
  assert.equal(setCalls[2].value.length, 0);
});

test("ChatInput does not restore stale send text into a switched conversation", async () => {
  const refs = [];
  const setCalls = [];
  const setDraftCalls = [];
  let resolveSend;
  const { tree } = renderChatInput({
    refs,
    setCalls,
    setDraftCalls,
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "thread a draft",
    draftKey: "thread-a",
    onSend: async () =>
      new Promise((resolve) => {
        resolveSend = () => resolve(null);
      }),
  });

  const textarea = findNode(tree, (node) =>
    node.strings.some((part) => part.includes("<textarea")),
  );
  const textareaProps = templateProps(textarea);
  textareaProps.onKeyDown({
    key: "Enter",
    shiftKey: false,
    preventDefault: () => {},
  });
  await Promise.resolve();

  const currentDraftKeyRef = refs[1];
  currentDraftKeyRef.current = "thread-b";
  resolveSend();
  await flushAsyncHandlers();

  assert.deepEqual(
    setCalls
      .filter((call) => call.index === 0)
      .map((call) => call.value),
    [""],
  );
  assert.deepEqual(setDraftCalls, [
    { key: "thread-a", text: "thread a draft" },
  ]);
});

test("ChatInput does not persist stale send text over a new same-thread draft", async () => {
  const refs = [];
  const setCalls = [];
  const setDraftCalls = [];
  let resolveSend;
  const { tree } = renderChatInput({
    refs,
    setCalls,
    setDraftCalls,
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "submitted draft",
    draftKey: "thread-a",
    onSend: async () =>
      new Promise((resolve) => {
        resolveSend = () => resolve(null);
      }),
  });

  const textarea = findNode(tree, (node) =>
    node.strings.some((part) => part.includes("<textarea")),
  );
  const textareaProps = templateProps(textarea);
  textareaProps.onKeyDown({
    key: "Enter",
    shiftKey: false,
    preventDefault: () => {},
  });
  await Promise.resolve();

  const textRef = refs[0];
  textRef.current = "new draft";
  resolveSend();
  await flushAsyncHandlers();

  assert.deepEqual(
    setCalls
      .filter((call) => call.index === 0)
      .map((call) => call.value),
    [""],
  );
  assert.deepEqual(setDraftCalls, []);
});

test("ChatInput does not restore submitted draft after auth scope changes", async () => {
  const setCalls = [];
  const setDraftCalls = [];
  let currentScope = "scope-a";
  let resolveSend;
  const { tree } = renderChatInput({
    setCalls,
    setDraftCalls,
    authScopeFn: () => currentScope,
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "private draft",
    draftKey: "thread-a",
    onSend: async () =>
      new Promise((resolve) => {
        resolveSend = () => resolve(null);
      }),
  });

  const textarea = findNode(tree, (node) =>
    node.strings.some((part) => part.includes("<textarea")),
  );
  const textareaProps = templateProps(textarea);
  textareaProps.onKeyDown({
    key: "Enter",
    shiftKey: false,
    preventDefault: () => {},
  });
  await Promise.resolve();

  currentScope = "scope-b";
  resolveSend();
  await flushAsyncHandlers();

  assert.deepEqual(
    setCalls
      .filter((call) => call.index === 0)
      .map((call) => call.value),
    [""],
  );
  assert.deepEqual(setDraftCalls, []);
});

test("ChatInput keeps Enter blocked when submit becomes disabled during send", async () => {
  const refs = [];
  let sendCalls = 0;
  let resolveSend;
  const { tree } = renderChatInput({
    refs,
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "draft while busy",
    onSend: async () =>
      new Promise((resolve) => {
        sendCalls += 1;
        resolveSend = () => resolve(null);
      }),
  });

  const textarea = findNode(tree, (node) =>
    node.strings.some((part) => part.includes("<textarea")),
  );
  const textareaProps = templateProps(textarea);
  textareaProps.onKeyDown({
    key: "Enter",
    shiftKey: false,
    preventDefault: () => {},
  });
  await Promise.resolve();

  // Re-render in production would update submitDisabledRef before the original
  // async send closure reaches finally.
  const submitDisabledRef = refs[5];
  submitDisabledRef.current = true;
  resolveSend();
  await flushAsyncHandlers();

  textareaProps.onKeyDown({
    key: "Enter",
    shiftKey: false,
    preventDefault: () => {},
  });
  await flushAsyncHandlers();

  assert.equal(sendCalls, 1);
});

// --- Command menu: keyboard-driven palette -----------------------------
// `menuSelection` ({index, dismissed}) is the 7th `useState` call in
// chat-input.tsx (slot index 6) — text(0)/attachments(1)/attachmentError(2)/
// isSending(3)/isCancelling(4)/dragOver(5) come first and their indices are
// pinned by the tests above, so the new state is appended after all of them
// rather than interleaved.
const MENU_SELECTION_STATE_INDEX = 6;

function findTextarea(tree) {
  return findNode(tree, (node) => node.strings.some((part) => part.includes("<textarea")));
}

function findCommandOption(tree, name) {
  return findNode(tree, (node) => node.props?.id === `chat-command-option-${name}`);
}

test("ChatInput ArrowDown moves the active command-menu row", () => {
  const setCalls = [];
  const { tree } = renderChatInput({
    setCalls,
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "/mo",
    commands: MENU_COMMANDS,
  });

  const textareaProps = templateProps(findTextarea(tree));
  let prevented = false;
  textareaProps.onKeyDown({
    key: "ArrowDown",
    preventDefault: () => {
      prevented = true;
    },
  });

  assert.equal(prevented, true);
  const menuSelectionCalls = setCalls.filter(
    (call) => call.index === MENU_SELECTION_STATE_INDEX,
  );
  assert.deepEqual(menuSelectionCalls, [
    { index: MENU_SELECTION_STATE_INDEX, value: { index: 1, dismissed: false } },
  ]);
});

test("ChatInput Enter completes the active command-menu row without sending", async () => {
  const setCalls = [];
  let sendCalls = 0;
  const { tree } = renderChatInput({
    setCalls,
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "/mo",
    commands: MENU_COMMANDS,
    onSend: async () => {
      sendCalls += 1;
    },
  });

  const textareaProps = templateProps(findTextarea(tree));
  let prevented = false;
  textareaProps.onKeyDown({
    key: "Enter",
    shiftKey: false,
    preventDefault: () => {
      prevented = true;
    },
  });
  await Promise.resolve();

  assert.equal(prevented, true);
  assert.equal(sendCalls, 0);
  const textCalls = setCalls.filter((call) => call.index === 0);
  assert.deepEqual(textCalls, [{ index: 0, value: "/model " }]);
});

test("ChatInput Tab completes the active command-menu row", async () => {
  const setCalls = [];
  let sendCalls = 0;
  const { tree } = renderChatInput({
    setCalls,
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "/mo",
    commands: MENU_COMMANDS,
    onSend: async () => {
      sendCalls += 1;
    },
  });

  const textareaProps = templateProps(findTextarea(tree));
  let prevented = false;
  textareaProps.onKeyDown({
    key: "Tab",
    preventDefault: () => {
      prevented = true;
    },
  });
  await Promise.resolve();

  assert.equal(prevented, true);
  assert.equal(sendCalls, 0);
  const textCalls = setCalls.filter((call) => call.index === 0);
  assert.deepEqual(textCalls, [{ index: 0, value: "/model " }]);
});

test("ChatInput Escape dismisses the command menu so a later Enter sends normally", async () => {
  const setCalls = [];
  let sendCalls = 0;
  const { tree } = renderChatInput({
    setCalls,
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "/mo",
    commands: MENU_COMMANDS,
    onSend: async () => {
      sendCalls += 1;
    },
  });

  const textareaProps = templateProps(findTextarea(tree));

  let escPrevented = false;
  textareaProps.onKeyDown({
    key: "Escape",
    preventDefault: () => {
      escPrevented = true;
    },
  });
  assert.equal(escPrevented, true);
  const menuSelectionCalls = setCalls.filter(
    (call) => call.index === MENU_SELECTION_STATE_INDEX,
  );
  assert.deepEqual(menuSelectionCalls, [
    { index: MENU_SELECTION_STATE_INDEX, value: { index: 0, dismissed: true } },
  ]);

  let enterPrevented = false;
  textareaProps.onKeyDown({
    key: "Enter",
    shiftKey: false,
    preventDefault: () => {
      enterPrevented = true;
    },
  });
  await Promise.resolve();

  assert.equal(enterPrevented, true);
  assert.equal(sendCalls, 1);
  // The only text-state change is handleSend's own clear-on-send; the draft
  // was never rewritten to "/model " by a completion, since Escape
  // suppressed the menu instead of Enter completing a row.
  const textCalls = setCalls.filter((call) => call.index === 0);
  assert.deepEqual(textCalls, [{ index: 0, value: "" }]);
});

test("ChatInput command-menu rows render the title and description", () => {
  const { tree } = renderChatInput({
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "/mo",
    commands: MENU_COMMANDS,
  });

  const modelRow = findCommandOption(tree, "model");
  const rowText = extractText(modelRow);
  assert.ok(rowText.includes(MENU_COMMANDS[0].title));
  assert.ok(rowText.includes(MENU_COMMANDS[0].description));
});

test("ChatInput command-menu shows the usage hint only for the active row", () => {
  const { tree } = renderChatInput({
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "/mo",
    commands: MENU_COMMANDS,
  });

  // Row 0 ("model") is active by default (no ArrowDown pressed yet).
  const activeRowText = extractText(findCommandOption(tree, "model"));
  assert.ok(activeRowText.includes(MENU_COMMANDS[0].usage));

  const inactiveRowText = extractText(findCommandOption(tree, "modelinfo"));
  assert.ok(!inactiveRowText.includes(MENU_COMMANDS[1].usage));
});

test("ChatInput command-menu highlights the typed prefix in the row's name", () => {
  const { tree } = renderChatInput({
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "/mo",
    commands: MENU_COMMANDS,
  });

  const modelRow = findCommandOption(tree, "model");
  const highlight = findNode(modelRow, (node) => node.props?.className === "text-signal");
  assert.ok(highlight, "expected a highlighted-prefix span inside the row");
  assert.equal(extractText(highlight), "mo");
  // The unhighlighted remainder of the name is still present, split apart
  // from the highlighted prefix rather than baked into one string.
  assert.equal(extractText(modelRow).includes("mo"), true);
  assert.equal(extractText(modelRow).includes("del"), true);
});

test("ChatInput Shift+Enter and Shift+Tab fall through the open command menu", async () => {
  const setCalls = [];
  let sendCalls = 0;
  const { tree } = renderChatInput({
    setCalls,
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "/mo",
    commands: MENU_COMMANDS,
    onSend: async () => {
      sendCalls += 1;
    },
  });

  const textareaProps = templateProps(findTextarea(tree));

  // Shift+Enter must behave exactly like the menu-closed case: native
  // newline insertion, not a command completion and not a send. `e.key` is
  // "Enter" regardless of the shift modifier, so the menu's own Enter/Tab
  // handling must explicitly check `!e.shiftKey`.
  let shiftEnterPrevented = false;
  textareaProps.onKeyDown({
    key: "Enter",
    shiftKey: true,
    preventDefault: () => {
      shiftEnterPrevented = true;
    },
  });
  await Promise.resolve();

  assert.equal(shiftEnterPrevented, false);
  assert.equal(sendCalls, 0);
  assert.deepEqual(setCalls.filter((call) => call.index === 0), []);

  // Shift+Tab must also fall through (not complete), for consistency.
  let shiftTabPrevented = false;
  textareaProps.onKeyDown({
    key: "Tab",
    shiftKey: true,
    preventDefault: () => {
      shiftTabPrevented = true;
    },
  });

  assert.equal(shiftTabPrevented, false);
  assert.deepEqual(setCalls.filter((call) => call.index === 0), []);
});

test("ChatInput Shift+Enter inserts a newline when the command menu is closed", async () => {
  let sendCalls = 0;
  const { tree } = renderChatInput({
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "plain text, no menu here",
    onSend: async () => {
      sendCalls += 1;
    },
  });

  const textareaProps = templateProps(findTextarea(tree));
  let prevented = false;
  textareaProps.onKeyDown({
    key: "Enter",
    shiftKey: true,
    preventDefault: () => {
      prevented = true;
    },
  });
  await Promise.resolve();

  assert.equal(prevented, false);
  assert.equal(sendCalls, 0);
});
