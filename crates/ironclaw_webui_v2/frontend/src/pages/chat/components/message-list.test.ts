// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

const messageListSource = readFileSync(
  new URL("./message-list.tsx", import.meta.url),
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
        .replace("export const JUMP_TO_LATEST_THRESHOLD_PX", "const JUMP_TO_LATEST_THRESHOLD_PX")
        .replace("export function distanceFromBottom", "function distanceFromBottom")
        .replace("export function isNearBottom", "function isNearBottom")
        .replace("export function shouldShowJumpToLatest", "function shouldShowJumpToLatest")
        .replace("export function scrollToBottom", "function scrollToBottom")
        .replace("export function messageKey", "function messageKey")
        .replace("export function isNewUserMessage", "function isNewUserMessage")
        .replace("export function MessageList", "function MessageList"),
    );
  }
  return `${lines.join("\n")}
globalThis.__testExports = {
  BOTTOM_FOLLOW_THRESHOLD_PX,
  JUMP_TO_LATEST_THRESHOLD_PX,
  FLOATING_CONTROL_BOTTOM_OFFSET_PX,
  FLOATING_CONTROL_STYLE,
  distanceFromBottom,
  isNearBottom,
  shouldShowJumpToLatest,
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
    "JUMP_TO_LATEST_THRESHOLD_PX",
    "distanceFromBottom",
    "isNearBottom",
    "shouldShowJumpToLatest",
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
  const {
    BOTTOM_FOLLOW_THRESHOLD_PX,
    JUMP_TO_LATEST_THRESHOLD_PX,
    isNearBottom,
    shouldShowJumpToLatest,
  } = loadHelpers();
  const viewport = {
    scrollHeight: 1000,
    scrollTop: 420,
    clientHeight: 500,
  };

  assert.equal(BOTTOM_FOLLOW_THRESHOLD_PX, 100);
  assert.equal(JUMP_TO_LATEST_THRESHOLD_PX, 240);
  assert.equal(
    isNearBottom(viewport),
    true,
    "a reader within the follow threshold should keep auto-scrolling",
  );
  assert.equal(
    shouldShowJumpToLatest(viewport),
    false,
    "the jump button should stay hidden while the reader is close to latest",
  );

  viewport.scrollTop = 300;

  assert.equal(
    isNearBottom(viewport),
    false,
    "a reader who scrolled up beyond the threshold should not be pulled down",
  );
  assert.equal(
    shouldShowJumpToLatest(viewport),
    false,
    "small scrollback should pause follow-scroll without showing the jump button",
  );

  viewport.scrollTop = 240;

  assert.equal(
    shouldShowJumpToLatest(viewport),
    true,
    "the jump button should appear only after meaningful scrollback",
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
    /const \[showJumpToLatest, setShowJumpToLatest\] = React\.useState\(false\);[\s\S]*const showJump = shouldShowJumpToLatest\(el\);[\s\S]*setShowJumpToLatest\(showJump\);/,
    "jump-to-latest visibility should use a larger threshold than follow-scroll",
  );
  assert.match(
    messageListSource,
    /if \(!shouldScrollRef\.current\) \{\s*setShowJumpToLatest\(shouldShowJumpToLatest\(el\)\);\s*return;\s*\}/,
    "paused follow-scroll should still reveal the jump button once new content creates meaningful distance",
  );
  assert.match(
    messageListSource,
    /else if \(userScrollIntentRef\.current\) \{[\s\S]*else \{[\s\S]*shouldScrollRef\.current = true;[\s\S]*followLatest\(\);/,
    "layout-driven scroll drift that is not upward should keep auto-follow enabled",
  );
  assert.match(
    messageListSource,
    /onWheel=\{markUserScrollIntent\}[\s\S]*onTouchMove=\{markUserScrollIntent\}[\s\S]*onPointerDown=\{markScrollbarDragIntent\}/,
    "only explicit scroll gestures should pause follow-scroll",
  );
  assert.match(
    messageListSource,
    /if \(!node \|\| \(!force && !shouldScrollRef\.current\)\) return;/,
    "resize-driven follow should still respect intentional user scrollback",
  );
});

test("MessageList renders a floating thread logs shortcut", () => {
  const {
    FLOATING_CONTROL_BOTTOM_OFFSET_PX,
    FLOATING_CONTROL_STYLE,
  } = loadHelpers();

  assert.match(
    messageListSource,
    /import \{ Link \} from "react-router";/,
    "thread logs shortcut should use React Router navigation",
  );
  assert.doesNotMatch(
    messageListSource,
    /buildScopedLogsPath/,
    "message-list should receive a logsPath prop instead of building routes",
  );
  assert.match(
    messageListSource,
    /logsPath,/,
    "message-list should accept a prebuilt thread logs route",
  );
  assert.match(
    messageListSource,
    /className="flex min-w-0 flex-1 overflow-y-auto overflow-x-hidden px-3 pt-5 pb-14 sm:px-5 sm:pt-6 lg:px-8"/,
    "scroll area should hide page-level horizontal overflow and keep normal bottom padding",
  );
  assert.match(
    messageListSource,
    /<div className="relative flex min-h-0 min-w-0 flex-1 overflow-hidden">/,
    "message-list should keep the transcript area as the floating-control anchor",
  );
  assert.doesNotMatch(
    messageListSource,
    /bottom-\[\$\{|h-\[\$\{|bottom-\[128px\]|h-\[164px\]/,
    "floating controls should not rely on Tailwind generated arbitrary classes",
  );
  assert.equal(FLOATING_CONTROL_BOTTOM_OFFSET_PX, 8);
  assert.equal(FLOATING_CONTROL_STYLE.bottom, 8);
  assert.doesNotMatch(
    messageListSource,
    /FLOATING_CONTROL_SPACER|className="hidden shrink-0 sm:block"/,
    "floating controls should use the scroll area's existing bottom padding instead of adding a visible gap after the last message",
  );
  assert.match(
    messageListSource,
    /const FLOATING_LOGS_BUTTON_CLASS =[\s\S]*group absolute right-5 z-10 hidden size-9[\s\S]*border-\[color-mix\(in_srgb,var\(--v2-accent\)_28%,var\(--v2-panel-border\)\)\][\s\S]*bg-\[color-mix\(in_srgb,var\(--v2-surface\)_88%,var\(--v2-accent\)_12%\)\][\s\S]*sm:inline-flex/,
    "floating logs button should be hidden on mobile and restore the desktop control at sm",
  );
  assert.match(
    messageListSource,
    /<Link\s+to=\{logsPath\}[\s\S]*className=\{FLOATING_LOGS_BUTTON_CLASS\}[\s\S]*style=\{FLOATING_CONTROL_STYLE\}[\s\S]*<Icon name="logs"/,
    "thread logs shortcut should render as a visible bottom-right icon button",
  );
  assert.match(
    messageListSource,
    /<Icon name="logs" className="size-5" \/>/,
    "thread logs icon should keep the desktop size because the control is hidden on mobile",
  );
  assert.match(
    messageListSource,
    /const JUMP_TO_BOTTOM_BUTTON_CLASS =[\s\S]*absolute left-1\/2 z-10 inline-flex max-w-\[calc\(100%-2rem\)\][\s\S]*items-center gap-1\.5 whitespace-nowrap rounded-full border border-\[var\(--v2-panel-border\)\][\s\S]*bg-\[var\(--v2-surface\)\] px-3 py-1\.5 text-xs font-medium text-\[var\(--v2-text-strong\)\][\s\S]*\{showJumpToLatest &&[\s\S]*className=\{JUMP_TO_BOTTOM_BUTTON_CLASS\}[\s\S]*style=\{FLOATING_CONTROL_STYLE\}/,
    "jump-to-latest should keep the pill style while using the composer-safe floating offset",
  );
});
