// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

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

function componentProps(node, component) {
  const props = {};
  const start = node.values.indexOf(component);
  for (let index = start + 1; index < node.values.length; index += 1) {
    const name = node.strings[index]?.match(/([A-Za-z][A-Za-z0-9]*)=\s*$/)?.[1];
    if (name) props[name] = node.values[index];
  }
  return props;
}

function templateProps(node) {
  const props = {};
  for (let index = 0; index < node.values.length; index += 1) {
    const name = node.strings[index]?.match(/([A-Za-z][A-Za-z0-9]*)=\s*$/)?.[1];
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
