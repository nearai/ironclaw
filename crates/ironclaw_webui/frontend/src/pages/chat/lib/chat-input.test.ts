// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";
import vm from "node:vm";

import {
  INITIAL_COMMAND_MENU_SELECTION,
  commandMenuMatches,
  commandMenuSelectionReducer,
  commandMenuToken,
} from "./chat-commands";
import {
  canStealFocus,
  shouldAutoFocusComposer,
} from "./chat-input-focus";
import {
  HTML_ATTRIBUTE_PATTERN,
  componentProps,
  componentSourceForTest,
  findComponent,
} from "../../../lib/vm-component-harness";

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
  return componentSourceForTest(
    new URL("../components/chat-input.tsx", import.meta.url),
    "ChatInput",
  );
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
    canStealFocus,
    shouldAutoFocusComposer,
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
      document: { activeElement: null },
      matchMedia: () => ({ matches: true }),
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

// Regression: the queued-steering composer swaps cancel for send as soon as the
// user types, so a follow-up can be queued behind the running turn. But when the
// send itself is unavailable (cooldown — the only `sendDisabled` cause that
// co-occurs with `canCancel`, since a gate/onboarding clears `canCancel`), that
// swap left a disabled send button and NO cancel: the user had to erase their
// draft to reach cancel. Keep cancel reachable whenever send cannot be used.
test("ChatInput keeps cancel reachable when a draft exists but send is disabled", () => {
  const { tree, components } = renderChatInput({
    disabled: false,
    sendDisabled: true,
    canCancel: true,
    draft: "follow-up while the run is cooling down",
  });

  const button = findComponent(tree, components.Button);
  const props = componentProps(button, components.Button);
  assert.equal(
    props["data-testid"],
    "chat-cancel-run",
    "a disabled send must not hide the cancel affordance behind clearing the draft",
  );
  assert.equal(props.disabled, false, "cancel stays clickable");
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

// A second, stateful hook harness used only by the thread-switch (item 4)
// test below. `renderChatInput`'s stub above is a fresh one-shot render
// (fine for every other test here, which each render exactly once); this
// one persists useState/useRef/useEffect slots by hook-call index — and,
// critically, actually MEMOIZES useCallback by its deps array — across
// repeated `render(props)` calls on the same instance, the same shape
// `useChat-send.test.ts`'s `createReactStub` uses. Real memoization matters
// here: `flushDraft` (deps `[]`) sits inside the draft-restore effect's own
// deps array, so an unmemoized (always-a-new-function) `useCallback` would
// make that effect look "changed" and re-fire on every render regardless of
// whether `draftKey` actually changed, defeating the test.
function createChatInputHookHost() {
  let stateIndex = 0;
  let refIndex = 0;
  let effectIndex = 0;
  let callbackIndex = 0;
  const stateSlots = [];
  const refSlots = [];
  const effectSlots = [];
  const callbackSlots = [];
  const setCalls = [];
  const refs = [];
  const depsChanged = (previous, next) => {
    if (!previous || !next || previous.length !== next.length) return true;
    return next.some((value, index) => !Object.is(value, previous[index]));
  };
  const React = {
    useCallback: (fn, deps) => {
      const index = callbackIndex++;
      const slot = callbackSlots[index];
      if (slot && !depsChanged(slot.deps, deps)) return slot.fn;
      callbackSlots[index] = { fn, deps: deps ? [...deps] : null };
      return fn;
    },
    useRef: (initial = null) => {
      const index = refIndex++;
      const ref = refSlots[index] || { current: initial };
      refSlots[index] = ref;
      if (!refs.includes(ref)) refs.push(ref);
      return ref;
    },
    useState: (initial) => {
      const index = stateIndex++;
      if (!(index in stateSlots)) {
        stateSlots[index] = typeof initial === "function" ? initial() : initial;
      }
      return [
        stateSlots[index],
        (next) => {
          stateSlots[index] =
            typeof next === "function" ? next(stateSlots[index]) : next;
          setCalls.push({ index, value: stateSlots[index] });
        },
      ];
    },
    useEffect: (effect, deps) => {
      const index = effectIndex++;
      const slot = effectSlots[index] || { deps: null, cleanup: null };
      if (!depsChanged(slot.deps, deps)) {
        effectSlots[index] = slot;
        return;
      }
      if (typeof slot.cleanup === "function") slot.cleanup();
      slot.deps = deps ? [...deps] : null;
      slot.cleanup = effect() || null;
      effectSlots[index] = slot;
    },
  };
  return { React, setCalls, refs, beginRender: () => {
    stateIndex = 0;
    refIndex = 0;
    effectIndex = 0;
    callbackIndex = 0;
  } };
}

function renderChatInputStateful({ getDraftByKey = {} } = {}) {
  const host = createChatInputHookHost();
  const components = { Button() {}, Icon() {} };
  const context = {
    ...components,
    React: host.React,
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    useT: () => (key) => key,
    authScope: () => "test-scope",
    stageFiles: async () => ({ staged: [], errors: [] }),
    commandMenuMatches,
    commandMenuToken,
    commandMenuSelectionReducer,
    INITIAL_COMMAND_MENU_SELECTION,
    canStealFocus,
    shouldAutoFocusComposer,
    useAttachmentConfig: () => ({
      accept: [],
      maxCount: 10,
      maxFileBytes: 1024,
      maxTotalBytes: 2048,
    }),
    NEW_DRAFT_KEY: "__new__",
    clearDraft: () => {},
    clearStagedAttachments: () => {},
    getDraft: (key) => getDraftByKey[key] || "",
    getStagedAttachments: () => [],
    setDraft: () => {},
    setStagedAttachments: () => {},
    window: {
      clearTimeout: () => {},
      document: { activeElement: null },
      matchMedia: () => ({ matches: true }),
      requestAnimationFrame: (fn) => fn(),
      setTimeout: () => 1,
    },
  };
  vm.runInNewContext(chatInputSourceForTest(), context);
  const ChatInputFn = context.globalThis.__testExports.ChatInput;
  return {
    components,
    host,
    // Exposed so a test can put focus on the control that navigated us here —
    // the real browser does exactly that, and stubbing it to `null` forever is
    // what let the "focus is never stolen from the sidebar button" bug ship.
    windowStub: context.window,
    render: (props) => {
      host.beginRender();
      return ChatInputFn({
        onSend: async () => {},
        disabled: false,
        sendDisabled: false,
        canCancel: false,
        commands: [],
        ...props,
      });
    },
  };
}

test("chat composer autofocus is desktop-only and survives a missing matchMedia", () => {
  assert.strictEqual(
    shouldAutoFocusComposer({ matchMedia: () => ({ matches: true }) }),
    true,
  );
  assert.strictEqual(
    shouldAutoFocusComposer({ matchMedia: () => ({ matches: false }) }),
    false,
  );
  assert.strictEqual(shouldAutoFocusComposer({}), false);
  assert.strictEqual(
    shouldAutoFocusComposer({
      matchMedia: () => {
        throw new Error("unavailable");
      },
    }),
    false,
  );
});

test("canStealFocus takes focus from the control that navigated here", () => {
  // The bug this pins: Chrome/Firefox focus a <button> on click, so after
  // clicking "+ New" or a sidebar thread row that button IS document
  // .activeElement when the composer's rAF runs. Refusing to steal from it
  // meant the composer was never focused on the two paths #7204 is about.
  const child = { tagName: "BUTTON", closest: () => null };
  const composer = {
    tagName: "TEXTAREA",
    contains: (node) => node === child,
  };
  const outside = (tagName, extra = {}) => ({
    tagName,
    closest: () => null,
    ...extra,
  });

  assert.strictEqual(canStealFocus(null, composer), true);
  assert.strictEqual(canStealFocus(outside("BODY"), composer), true);
  assert.strictEqual(canStealFocus(composer, composer), true);
  assert.strictEqual(canStealFocus(child, composer), true);
  assert.strictEqual(canStealFocus(outside("BUTTON"), composer), true);
  assert.strictEqual(canStealFocus(outside("A"), composer), true);

  // Deliberate text entry elsewhere, and any modal focus trap, still win.
  assert.strictEqual(canStealFocus(outside("INPUT"), composer), false);
  assert.strictEqual(canStealFocus(outside("TEXTAREA"), composer), false);
  assert.strictEqual(canStealFocus(outside("SELECT"), composer), false);
  assert.strictEqual(
    canStealFocus(outside("DIV", { isContentEditable: true }), composer),
    false,
  );
  assert.strictEqual(
    canStealFocus(
      { tagName: "BUTTON", closest: (selector) => ({ selector }) },
      composer,
    ),
    false,
  );
});

test("ChatInput focuses restored drafts only when composer identity changes", () => {
  const { render, windowStub } = renderChatInputStateful({
    getDraftByKey: {
      "thread-a": "first",
      "thread-b": "restored draft",
      "thread-e": "history restored",
    },
  });
  const focusCalls = [];
  const composer = {
    style: {},
    scrollHeight: 40,
    focus: () => focusCalls.push("focus"),
    setSelectionRange: (start, end) => focusCalls.push([start, end]),
    contains: () => false,
  };
  // What a real browser hands us after the click that navigated here.
  const sidebarButton = { tagName: "BUTTON", closest: () => null };

  const tree = render({ draftKey: "thread-a", resetKey: "route-a" });
  templateProps(findTextarea(tree)).ref.current = composer;

  // Opening "thread-b" from the sidebar: the row button holds focus, and the
  // composer must take it and land the caret at the end of the restored draft.
  windowStub.document.activeElement = sidebarButton;
  render({ draftKey: "thread-b", resetKey: "route-b" });
  assert.deepStrictEqual(focusCalls, ["focus", [14, 14]]);

  // Same identity re-render (a keystroke, an SSE frame): no refocus, no caret move.
  render({ draftKey: "thread-b", resetKey: "route-b" });
  assert.deepStrictEqual(focusCalls, ["focus", [14, 14]]);

  // A hard-disabled composer is never focused.
  render({ draftKey: "thread-c", resetKey: "route-c", disabled: true });
  assert.deepStrictEqual(focusCalls, ["focus", [14, 14]]);

  // Deliberate text entry elsewhere keeps focus.
  windowStub.document.activeElement = { tagName: "INPUT", closest: () => null };
  render({ draftKey: "thread-d", resetKey: "route-d" });
  assert.deepStrictEqual(focusCalls, ["focus", [14, 14]]);

  // Browser history can leave the mounted composer focused while restoring a
  // different route's draft. Its stale selection still moves to the new end.
  windowStub.document.activeElement = composer;
  render({ draftKey: "thread-e", resetKey: "route-e" });
  assert.deepStrictEqual(focusCalls, ["focus", [14, 14], [16, 16]]);

  // The first reply can adopt its thread id without changing location. Keep
  // the user's active selection in that same-route case.
  render({ draftKey: "thread-e-adopted", resetKey: "route-e" });
  assert.deepStrictEqual(focusCalls, ["focus", [14, 14], [16, 16]]);
});

test("ChatInput focuses a handed-off landing draft with the caret at its end", () => {
  // The landing-hero -> thread hand-off used to own the only focus call. It
  // now rides the shared focus effect, so it needs its own assertion that the
  // caret still lands after the handed-off text rather than at offset 0.
  const { render, windowStub } = renderChatInputStateful();
  const focusCalls = [];
  const composer = {
    style: {},
    scrollHeight: 40,
    focus: () => focusCalls.push("focus"),
    setSelectionRange: (start, end) => focusCalls.push([start, end]),
    contains: () => false,
  };

  const tree = render({ draftKey: "__new__", resetKey: "route-a" });
  templateProps(findTextarea(tree)).ref.current = composer;

  windowStub.document.activeElement = { tagName: "BUTTON", closest: () => null };
  render({
    draftKey: "thread-new",
    resetKey: "route-b",
    initialText: "handed off",
  });
  assert.deepStrictEqual(focusCalls, ["focus", [10, 10]]);
});

test("ChatInput removes the container focus ring but keeps textarea neutralizers", () => {
  const { tree } = renderChatInput({ disabled: false });
  const textarea = findTextarea(tree);
  const textareaClass = templateProps(textarea).className;
  const allClassNames = [];
  findNode(tree, (node) => {
    const className = templateProps(node).className;
    if (typeof className === "string") allClassNames.push(className);
    return false;
  });

  // Anchor first: without this the absence assertion below passes vacuously
  // the day the harness stops capturing `className`.
  assert.strictEqual(
    allClassNames.some((name) => name.includes("rounded-[20px]")),
    true,
    "composer container className must be reachable for this test to mean anything",
  );
  assert.strictEqual(
    allClassNames.some((name) => name.includes("focus-within:")),
    false,
  );
  // These suppress the global `input:focus` accent in styles/app.css. Deleting
  // them re-draws the same ring, tighter, around the textarea itself.
  assert.strictEqual(textareaClass.includes("focus:!shadow-none"), true);
  assert.strictEqual(textareaClass.includes("focus:!border-transparent"), true);
});

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
  const refs = [];
  let sendCalls = 0;
  const { tree } = renderChatInput({
    setCalls,
    refs,
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
  // Regression: completion must queue the same debounced draft persist
  // handleChange uses, so a thread switch right after Enter doesn't restore
  // the stale pre-completion prefix (see completeMenuCommand). Copy the
  // primitive fields out before comparing — the pending-draft object is
  // constructed inside the vm-sandboxed component source, so a direct
  // `deepEqual` against an object literal from this (outer-realm) test file
  // would fail on cross-realm prototype identity despite equal values.
  const pendingDraft = refs[9].current;
  assert.deepEqual(
    { key: pendingDraft.key, text: pendingDraft.text, scope: pendingDraft.scope },
    { key: "__new__", text: "/model ", scope: "test-scope" },
  );
});

test("ChatInput Tab completes the active command-menu row", async () => {
  const setCalls = [];
  const refs = [];
  let sendCalls = 0;
  const { tree } = renderChatInput({
    setCalls,
    refs,
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
  // Regression: same queued-persist guarantee via the Tab completion path
  // (see the Enter test above for why the fields are copied before compare).
  const pendingDraft = refs[9].current;
  assert.deepEqual(
    { key: pendingDraft.key, text: pendingDraft.text, scope: pendingDraft.scope },
    { key: "__new__", text: "/model ", scope: "test-scope" },
  );
});

test("ChatInput Enter submits an exact single command-menu match instead of completing it again", async () => {
  // Regression (item 3): a draft that's already an exact, sole match for one
  // command ("/status" against a menu whose only row is "status") kept the
  // menu open, so Enter rewrote the draft to "/status " (a no-op completion)
  // and the user had to press Enter twice. Enter must submit here instead —
  // Tab still completes (covered separately below).
  const setCalls = [];
  const sentContents = [];
  let sendCalls = 0;
  const { tree } = renderChatInput({
    setCalls,
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "/status",
    commands: MENU_COMMANDS,
    onSend: async (content) => {
      sendCalls += 1;
      sentContents.push(content);
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
  await flushAsyncHandlers();

  // Still prevented — by the bottom Enter-to-send handler, not the menu's
  // completion branch.
  assert.equal(prevented, true);
  assert.equal(sendCalls, 1, "Enter on an exact single match must submit");
  assert.deepEqual(sentContents, ["/status"]);
  // The draft was submitted (and cleared by handleSend's own send-start
  // reset), not rewritten to the completed "/status " form.
  const textCalls = setCalls.filter((call) => call.index === 0);
  assert.deepEqual(textCalls, [{ index: 0, value: "" }]);
});

test("ChatInput Enter still completes a single command-menu match that is only a partial prefix", async () => {
  // The critical boundary the fix above must not overshoot: a menu with
  // exactly one row is not by itself an exact match — "/stat" also narrows
  // MENU_COMMANDS to the single "status" row, but the draft is still a
  // partial prefix, so Enter must complete it (not submit "/stat").
  const setCalls = [];
  let sendCalls = 0;
  const { tree } = renderChatInput({
    setCalls,
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "/stat",
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
  assert.equal(sendCalls, 0, "a partial single match must still complete, not submit");
  const textCalls = setCalls.filter((call) => call.index === 0);
  assert.deepEqual(textCalls, [{ index: 0, value: "/status " }]);
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

test("ChatInput resets a dismissed command menu when the draft key changes", () => {
  // Regression (item 4): `menuSelection`/`commandTokenRef` were reset only
  // from `handleChange`. The draft-restore effect sets `text` programmatically
  // on a thread switch without going through that reset, so an Esc-dismissal
  // on thread A hid the menu on thread B until the first keystroke there.
  const { render } = renderChatInputStateful({
    getDraftByKey: { "thread-a": "/mo", "thread-b": "/mo" },
  });

  // Render #1: mount on thread-a with a menu-matching draft.
  let tree = render({ draftKey: "thread-a", commands: MENU_COMMANDS });
  assert.ok(
    findCommandOption(tree, "model"),
    "menu open on thread-a before dismissal",
  );

  // Dismiss it (Esc) — same handler exercised by the Escape test above.
  const textareaProps = templateProps(findTextarea(tree));
  textareaProps.onKeyDown({ key: "Escape", preventDefault: () => {} });

  // Re-render the SAME draftKey (mirrors React committing the Esc setState)
  // to observe the dismissal actually took effect.
  tree = render({ draftKey: "thread-a", commands: MENU_COMMANDS });
  assert.equal(
    findCommandOption(tree, "model"),
    null,
    "menu dismissed on thread-a",
  );

  // Switch to thread-b: draftKey changes, so the draft-restore effect fires.
  render({ draftKey: "thread-b", commands: MENU_COMMANDS });
  // The effect's own setState call lands after this render function
  // returns (mirrors real React's post-commit effect timing, not a
  // same-render synchronous update) — render once more with the same props
  // to observe the settled post-effect state.
  tree = render({ draftKey: "thread-b", commands: MENU_COMMANDS });

  assert.ok(
    findCommandOption(tree, "model"),
    "the menu must be available again on thread-b, not suppressed by thread-a's dismissal",
  );
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

test("ChatInput command-menu footer shows the usage hint for the active row only", () => {
  // The usage hint moved from an inline line inside the active row (which
  // made that one row taller than its neighbors — see the visual-design
  // pass on the command menu) to a single footer slot shared by whichever
  // row is active. This pins the same behavior the row-level version
  // protected: exactly one command's usage is visible, and it's the active
  // row's, never an inactive row's.
  const { tree } = renderChatInput({
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "/mo",
    commands: MENU_COMMANDS,
  });

  // Row 0 ("model") is active by default (no ArrowDown pressed yet).
  const usageHint = findNode(
    tree,
    (node) => node.props?.["data-testid"] === "chat-command-menu-usage",
  );
  assert.ok(usageHint, "expected a usage hint in the command-menu footer");
  assert.equal(extractText(usageHint), MENU_COMMANDS[0].usage);
  assert.notEqual(extractText(usageHint), MENU_COMMANDS[1].usage);

  // Rows themselves no longer render usage text at all.
  const modelRow = findCommandOption(tree, "model");
  assert.equal(extractText(modelRow).includes(MENU_COMMANDS[0].usage), false);
});

test("ChatInput command-menu footer usage hint follows the active row after ArrowDown", () => {
  const { render } = renderChatInputStateful({
    getDraftByKey: { thread: "/mo" },
  });

  let tree = render({ draftKey: "thread", commands: MENU_COMMANDS });
  const usageAt = (currentTree) =>
    extractText(
      findNode(
        currentTree,
        (node) => node.props?.["data-testid"] === "chat-command-menu-usage",
      ),
    );
  assert.equal(usageAt(tree), MENU_COMMANDS[0].usage);

  templateProps(findTextarea(tree)).onKeyDown({
    key: "ArrowDown",
    preventDefault: () => {},
  });
  tree = render({ draftKey: "thread", commands: MENU_COMMANDS });

  assert.equal(usageAt(tree), MENU_COMMANDS[1].usage);
});

test("ChatInput command-menu header shows a label and the match count out of the full inventory", () => {
  const { tree } = renderChatInput({
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "/mo",
    commands: MENU_COMMANDS,
  });

  // "mo" matches "model" and "modelinfo" (not "status") out of 3 total.
  const count = findNode(
    tree,
    (node) => node.props?.["data-testid"] === "chat-command-menu-count",
  );
  assert.ok(count, "expected a match-count badge in the command-menu header");
  assert.equal(extractText(count), "2/3");

  // `useT` is stubbed to the identity function in this harness, so the
  // translated header label renders as its own key.
  const label = findNode(tree, (node) => extractText(node) === "chat.commandMenu");
  assert.ok(label, "expected the header label to render the commandMenu copy");
});

test("ChatInput command-menu footer renders the navigate/run/complete/dismiss key legend", () => {
  const { tree } = renderChatInput({
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "/mo",
    commands: MENU_COMMANDS,
  });

  // The keycap glyphs are literal (not translated), matching how
  // command-palette.tsx's own "esc" keycap is untranslated.
  for (const glyph of ["↑↓", "↵", "Tab", "Esc"]) {
    const kbd = findNode(tree, (node) => node.type === "kbd" && extractText(node) === glyph);
    assert.ok(kbd, `expected a <kbd> hint for "${glyph}"`);
  }
});

test("ChatInput command-menu shows an intentional empty state for a bare prefix with no matches", () => {
  const { tree } = renderChatInput({
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "/zz",
    commands: MENU_COMMANDS,
  });

  const empty = findNode(
    tree,
    (node) => node.props?.["data-testid"] === "chat-command-menu-empty",
  );
  assert.ok(empty, "expected an intentional empty state, not a silently empty popover");
  // Reuses the same copy key the ⌘K command-palette's own empty state uses.
  assert.equal(extractText(empty), "command.noMatches");
  assert.equal(findCommandOption(tree, "model"), null);

  // The header count reflects zero matches out of the full inventory.
  const count = findNode(
    tree,
    (node) => node.props?.["data-testid"] === "chat-command-menu-count",
  );
  assert.equal(extractText(count), "0/3");
});

test("ChatInput command-menu renders nothing for plain text that isn't a bare command prefix", () => {
  const { tree } = renderChatInput({
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "plain text, no menu here",
    commands: MENU_COMMANDS,
  });

  assert.equal(
    findNode(tree, (node) => node.props?.["data-testid"] === "chat-command-menu-empty"),
    null,
  );
  assert.equal(findCommandOption(tree, "model"), null);
});

test("ChatInput Escape dismisses the empty no-match command-menu state", () => {
  const setCalls = [];
  const { tree } = renderChatInput({
    setCalls,
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "/zz",
    commands: MENU_COMMANDS,
  });

  const textareaProps = templateProps(findTextarea(tree));
  let prevented = false;
  textareaProps.onKeyDown({
    key: "Escape",
    preventDefault: () => {
      prevented = true;
    },
  });

  assert.equal(prevented, true);
  const menuSelectionCalls = setCalls.filter(
    (call) => call.index === MENU_SELECTION_STATE_INDEX,
  );
  assert.deepEqual(menuSelectionCalls, [
    { index: MENU_SELECTION_STATE_INDEX, value: { index: 0, dismissed: true } },
  ]);
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

test("ChatInput command-menu rows are non-focusable listbox options, not tab stops", () => {
  // Regression (item 7, a11y): a focusable <button role="option"> inside
  // role="listbox" is a second, competing tab stop alongside the textarea's
  // own aria-activedescendant-driven selection, breaking the listbox
  // pattern. Rows must render as a non-focusable element (mouse handlers
  // preserved) so the textarea remains the only stop.
  const { tree } = renderChatInput({
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "/mo",
    commands: MENU_COMMANDS,
  });

  const modelRow = findCommandOption(tree, "model");
  assert.ok(modelRow, "expected a command-menu row for 'model'");
  assert.equal(
    modelRow.type,
    "div",
    "command-menu rows must not render as a focusable <button>",
  );
  assert.equal(modelRow.props.role, "option");
  assert.equal(typeof modelRow.props.onClick, "function", "click-to-complete must still be wired");
  assert.equal(typeof modelRow.props.onMouseEnter, "function", "hover-to-select must still be wired");
  assert.equal(typeof modelRow.props.onMouseDown, "function", "focus-steal prevention must still be wired");
});

test("ChatInput textarea advertises combobox semantics only while the command menu is open", () => {
  // Regression (a11y): the textarea already wired aria-expanded/aria-controls/
  // aria-activedescendant, but without role="combobox" and
  // aria-autocomplete="list" a screen reader has no reason to announce those
  // as combobox state rather than stray attributes on a plain textbox. Both
  // new attributes must follow the exact same `menuVisible` guard as
  // aria-controls: present — and pointing at a listbox that actually renders
  // — only while the menu is in play, absent (not a dangling reference)
  // otherwise. See the WAI-ARIA "Editable Combobox With List Autocomplete"
  // pattern.
  const open = renderChatInput({
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "/mo",
    commands: MENU_COMMANDS,
  });
  const openProps = templateProps(findTextarea(open.tree));
  assert.equal(openProps.role, "combobox");
  assert.equal(openProps["aria-autocomplete"], "list");
  assert.equal(openProps["aria-expanded"], true);
  assert.equal(openProps["aria-controls"], "chat-command-menu-listbox");
  assert.equal(openProps["aria-activedescendant"], "chat-command-option-model");
  assert.ok(
    findNode(open.tree, (node) => node.props?.id === "chat-command-menu-listbox"),
    "the listbox aria-controls names must actually be rendered, not a dangling reference",
  );

  const closed = renderChatInput({
    disabled: false,
    sendDisabled: false,
    canCancel: false,
    draft: "plain text, no menu here",
    commands: MENU_COMMANDS,
  });
  const closedProps = templateProps(findTextarea(closed.tree));
  assert.equal(closedProps.role, undefined);
  assert.equal(closedProps["aria-autocomplete"], undefined);
  assert.equal(closedProps["aria-expanded"], false);
  assert.equal(closedProps["aria-controls"], undefined);
  assert.equal(closedProps["aria-activedescendant"], undefined);
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
