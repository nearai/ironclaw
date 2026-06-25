import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const messageListSource = readFileSync(
  new URL("./message-list.js", import.meta.url),
  "utf8",
);

function messageListSourceForTest() {
  const lines = [];
  let skippingImport = false;
  for (const line of messageListSource.split("\n")) {
    if (!skippingImport && line.startsWith("import ")) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    if (skippingImport) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    lines.push(
      line
        .replace("export const BOTTOM_FOLLOW_THRESHOLD_PX", "const BOTTOM_FOLLOW_THRESHOLD_PX")
        .replace("export function distanceFromBottom", "function distanceFromBottom")
        .replace("export function isNearBottom", "function isNearBottom")
        .replace("export function scrollToBottom", "function scrollToBottom")
        .replace("export function messageKey", "function messageKey")
        .replace("export function isNewUserMessage", "function isNewUserMessage")
        .replace("export function MessageList", "function MessageList"),
    );
  }
  return `${lines.join("\n")}
globalThis.__testExports = {
  BOTTOM_FOLLOW_THRESHOLD_PX,
  distanceFromBottom,
  isNearBottom,
  scrollToBottom,
  messageKey,
  isNewUserMessage,
};`;
}

function loadHelpers() {
  const context = { globalThis: {}, Number };
  vm.runInNewContext(messageListSourceForTest(), context);
  return context.globalThis.__testExports;
}

test("MessageList keeps scroll helpers exported", () => {
  for (const name of [
    "BOTTOM_FOLLOW_THRESHOLD_PX",
    "distanceFromBottom",
    "isNearBottom",
    "scrollToBottom",
    "messageKey",
    "isNewUserMessage",
  ]) {
    assert.match(
      messageListSource,
      new RegExp(`export (?:const|function) ${name}\\b`),
      `${name} should remain importable by regression tests`,
    );
  }
});

test("MessageList follows streamed content only while near the bottom", () => {
  const { isNearBottom } = loadHelpers();
  const viewport = {
    scrollHeight: 1000,
    scrollTop: 420,
    clientHeight: 500,
  };

  assert.equal(
    isNearBottom(viewport),
    true,
    "a reader within the follow threshold should keep auto-scrolling",
  );

  viewport.scrollTop = 300;

  assert.equal(
    isNearBottom(viewport),
    false,
    "a reader who scrolled up beyond the threshold should not be pulled down",
  );
});

test("MessageList scrollToBottom pins the viewport to the latest content", () => {
  const { distanceFromBottom, scrollToBottom } = loadHelpers();
  const viewport = {
    scrollHeight: 1600,
    scrollTop: 500,
    clientHeight: 600,
  };

  scrollToBottom(viewport);

  assert.equal(viewport.scrollTop, 1000);
  assert.equal(distanceFromBottom(viewport), 0);
});

test("MessageList force-follows when the latest message is newly sent by the user", () => {
  const { isNewUserMessage, messageKey } = loadHelpers();
  const previousAssistant = { id: "reply-1", role: "assistant" };
  const nextUser = { id: "pending-1", role: "user" };
  const nextAssistant = { id: "reply-2", role: "assistant" };

  assert.equal(
    isNewUserMessage(messageKey(previousAssistant), nextUser),
    true,
    "sending a new user message should force the chat to the bottom",
  );
  assert.equal(
    isNewUserMessage(messageKey(nextUser), nextUser),
    false,
    "re-rendering the same user message should not keep forcing scroll",
  );
  assert.equal(
    isNewUserMessage(messageKey(nextUser), nextAssistant),
    false,
    "assistant streaming should still respect intentional user scrollback",
  );
});

test("MessageList observes content growth from streamed markdown layout", () => {
  assert.match(
    messageListSource,
    /const force = isNewUserMessage\(latestMessageKeyRef\.current, latestMessage\);[\s\S]*followLatest\(force\);/,
    "streaming updates should follow before the browser paints the next chunk",
  );
  assert.match(
    messageListSource,
    /const rafRef = React\.useRef\(null\);[\s\S]*window\.cancelAnimationFrame\(rafRef\.current\);[\s\S]*rafRef\.current = window\.requestAnimationFrame/,
    "follow-scroll should keep a single active animation frame",
  );
  assert.match(
    messageListSource,
    /new ResizeObserver\(\(\) => \{\s*followLatest\(\);/s,
    "post-render content growth should keep following while the reader is at the bottom",
  );
  assert.match(
    messageListSource,
    /const scrollRafRef = React\.useRef\(null\);[\s\S]*const syncScrollState = React\.useCallback[\s\S]*window\.requestAnimationFrame\(syncScrollState\);/,
    "scroll events should update the follow guard immediately but throttle rendered state updates",
  );
  assert.match(
    messageListSource,
    /const previousScrollTopRef = React\.useRef\(0\);[\s\S]*const isUpwardScroll = el\.scrollTop < previousScrollTopRef\.current;[\s\S]*if \(!nearBottom && isUpwardScroll\) \{[\s\S]*userScrollIntentRef\.current = true;[\s\S]*else if \(userScrollIntentRef\.current\) \{[\s\S]*shouldScrollRef\.current = false;/,
    "unattributed upward scrolls away from the bottom should pause follow-scroll",
  );
  assert.match(
    messageListSource,
    /else if \(userScrollIntentRef\.current\) \{[\s\S]*else \{[\s\S]*shouldScrollRef\.current = true;[\s\S]*followLatest\(\);/,
    "layout-driven scroll drift that is not upward should keep auto-follow enabled",
  );
  assert.match(
    messageListSource,
    /onWheel=\$\{markUserScrollIntent\}[\s\S]*onTouchMove=\$\{markUserScrollIntent\}[\s\S]*onPointerDown=\$\{markScrollbarDragIntent\}/,
    "only explicit scroll gestures should pause follow-scroll",
  );
  assert.match(
    messageListSource,
    /if \(!node \|\| \(!force && !shouldScrollRef\.current\)\) return;/,
    "resize-driven follow should still respect intentional user scrollback",
  );
});
