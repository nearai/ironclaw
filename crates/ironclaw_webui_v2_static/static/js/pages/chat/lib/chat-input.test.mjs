import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function chatInputSourceForTest() {
  const source = readFileSync(
    new URL("../components/chat-input.js", import.meta.url),
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
    authScope: () => "test-scope",
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
    setDraft: () => {},
    setStagedAttachments: () => {},
    window: { requestAnimationFrame: (fn) => fn() },
  };

  vm.runInNewContext(chatInputSourceForTest(), context);
  const tree = context.globalThis.__testExports.ChatInput({
    onSend,
    onCancel,
    disabled,
    sendDisabled,
    canCancel,
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
  assert.equal(
    setCalls.some((call) => call.index === 0 && call.value === ""),
    false,
  );
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
  const submitDisabledRef = refs[3];
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
