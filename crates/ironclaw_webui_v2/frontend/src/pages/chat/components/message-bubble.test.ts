import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { test, vi } from "vitest";

import {
  CHAT_MESSAGE_ROLES,
  type ErrorChatMessage,
} from "../lib/message-types";

vi.mock("./markdown-renderer", async () => {
  const { createElement } = await import("react");
  return {
    MarkdownRenderer: ({ content, className }) =>
      createElement("div", { className, "data-testid": "markdown" }, content),
  };
});

vi.mock("./tool-activity", async () => {
  const { createElement } = await import("react");
  return {
    ToolActivity: () => createElement("div", { "data-testid": "tool-activity" }),
  };
});

vi.mock("../../../design-system/icons", async () => {
  const { createElement } = await import("react");
  return {
    Icon: ({ name, className }) =>
      createElement("span", { className, "data-icon": name }),
  };
});

vi.mock("../../../lib/toast", () => ({ toast: () => {} }));
vi.mock("../../../lib/i18n", () => ({ useT: () => (key) => key }));

vi.mock("./project-file-chips", async () => {
  const { createElement } = await import("react");
  return {
    ProjectFileChips: () =>
      createElement("div", { "data-testid": "project-file-chips" }),
  };
});

vi.mock("./attachment-chip", async () => {
  const { createElement } = await import("react");
  return {
    AttachmentChip: () => createElement("div", { "data-testid": "attachment-chip" }),
  };
});

vi.mock("./attachment-preview", async () => {
  const { createElement } = await import("react");
  return {
    AttachmentPreviewModal: () =>
      createElement("div", { "data-testid": "attachment-preview" }),
  };
});

const messageBubbleSource = readFileSync(
  new URL("./message-bubble.tsx", import.meta.url),
  "utf8",
);
const appCssSource = readFileSync(
  new URL("../../../styles/app.css", import.meta.url),
  "utf8",
);

test("conversation message bubbles use readable typography", () => {
  assert.match(
    messageBubbleSource,
    /['"`]text-base\s+leading-7['"`]/,
    "chat message content should render at a readable base size",
  );
  assert.doesNotMatch(
    messageBubbleSource,
    /['"`]text-sm\s+leading-6['"`]/,
    "chat message content should not regress to the compact body size",
  );
});

test("assistant bubbles expose final reply state for live QA", () => {
  assert.match(
    messageBubbleSource,
    /const finalReplyState =[\s\S]*message\.isFinalReply/,
    "assistant messages should derive a DOM-readable final reply state",
  );
  assert.match(
    messageBubbleSource,
    /data-final-reply=\{finalReplyState\}/,
    "live QA should be able to distinguish streaming text from the final answer",
  );
});

test("markdown body and code blocks inherit readable message sizing", () => {
  assert.match(
    appCssSource,
    /\.markdown-body\s*\{[^}]*font-size:\s*1em;[^}]*line-height:\s*1\.7;/,
    "markdown prose should inherit the message bubble size with readable leading",
  );
  assert.match(
    appCssSource,
    /\.markdown-body\s+pre\s+code\s*\{[^}]*font-size:\s*0\.9em;\s*line-height:\s*1\.65;/,
    "fenced code should stay close to body size instead of shrinking below readability",
  );
  assert.match(
    appCssSource,
    /\.markdown-body\s*\{[^}]*overflow-wrap:\s*anywhere;/,
    "markdown prose should wrap long inline tokens on narrow screens",
  );
  assert.doesNotMatch(
    appCssSource,
    /word-break:\s*break-word;/,
    "overflow-wrap:anywhere should not be paired with deprecated word-break:break-word",
  );
  assert.match(
    appCssSource,
    /\.markdown-body\s+pre\s*\{[^}]*overflow-wrap:\s*normal;[^}]*word-break:\s*normal;/,
    "fenced code should keep its own horizontal scroll instead of forcing global page overflow",
  );
  assert.match(
    appCssSource,
    /\.markdown-body\s+table\s*\{[^}]*table-layout:\s*fixed;/,
    "markdown tables should fit the message column instead of expanding the viewport",
  );
});

test("conversation bubbles use mobile-safe shared widths and wrap long user tokens", () => {
  assert.match(
    appCssSource,
    /--v2-chat-readable-max-width:\s*[^;]+;/,
    "chat readable width should be defined once as a CSS token",
  );
  assert.match(
    appCssSource,
    /\.v2-chat-readable-width\s*\{[^}]*max-width:\s*100%;/,
    "chat readable width should default to the full mobile column",
  );
  assert.match(
    appCssSource,
    /@media\s*\(min-width:\s*640px\)\s*\{[\s\S]*\.v2-chat-readable-width\s*\{[^}]*max-width:\s*var\(--v2-chat-readable-max-width\);/,
    "chat readable width should align its desktop breakpoint with Tailwind sm",
  );
  assert.match(
    appCssSource,
    /@media\s*\(max-width:\s*639\.98px\)\s*\{[\s\S]*\.markdown-body\s+table/,
    "mobile markdown overrides should stop before Tailwind sm begins",
  );
  assert.doesNotMatch(
    appCssSource,
    /@media\s*\(max-width:\s*768px\)/,
    "mobile markdown overrides should not overlap Tailwind sm viewports",
  );
  assert.match(
    messageBubbleSource,
    /\? "v2-chat-readable-width"/,
    "user bubbles should use the shared readable width utility",
  );
  assert.match(
    messageBubbleSource,
    /: "w-full v2-chat-readable-width";/,
    "assistant bubbles should use the shared readable width utility",
  );
  assert.doesNotMatch(
    messageBubbleSource,
    /sm:max-w-\[[^\]]+\]/,
    "message bubbles should not scatter desktop width constants in component strings",
  );
  assert.match(
    messageBubbleSource,
    /className="v2-wrap-anywhere whitespace-pre-wrap break-words"/,
    "plain user text should break long unbroken strings",
  );
});

test("error messages render as inline chat bubbles, not centered notices", async () => {
  const { MessageBubble } = await import("./message-bubble");
  const html = renderToStaticMarkup(
    React.createElement(MessageBubble, {
      message: {
        id: "err-1",
        role: CHAT_MESSAGE_ROLES.ERROR,
        content: "Provider unavailable",
        timestamp: "2026-06-02T00:00:00.000Z",
      },
      onRetry: () => {},
      threadId: "thread-1",
    }),
  );

  assert.match(
    html,
    /data-testid="msg-error"/,
    "error role should render through the message bubble path",
  );
  assert.match(
    html,
    /mr-auto[^"]*v2-chat-readable-width/,
    "error bubbles should use a compact readable-width bubble instead of a full-width centered notice",
  );
  assert.match(
    html,
    /mr-auto[^"]*text-left text-red-200/,
    "error role should align with the assistant-side chat stream",
  );
  assert.doesNotMatch(
    html,
    /mx-auto[^"]*text-center/,
    "error role must not regress to the old centered banner styling",
  );
  assert.match(html, /Provider unavailable/);
  assert.doesNotMatch(html, /data-failure-category=/);
  assert.doesNotMatch(html, /data-failure-status=/);
});

test("error bubbles expose structural provider failure metadata", async () => {
  const { MessageBubble } = await import("./message-bubble");
  const message: ErrorChatMessage = {
    id: "err-provider-unavailable",
    role: CHAT_MESSAGE_ROLES.ERROR,
    content: "Provider unavailable",
    timestamp: "2026-07-12T00:00:00.000Z",
    failureCategory: "model_unavailable",
    failureStatus: "failed",
  };

  const html = renderToStaticMarkup(
    React.createElement(MessageBubble, {
      message,
      onRetry: () => {},
      threadId: "thread-1",
    }),
  );

  assert.match(html, /data-failure-category="model_unavailable"/);
  assert.match(html, /data-failure-status="failed"/);
});

test("message timestamp and actions share a hover-only meta row", () => {
  assert.match(
    messageBubbleSource,
    /const showActions =[\s\S]*CHAT_MESSAGE_ROLES\.USER[\s\S]*CHAT_MESSAGE_ROLES\.ASSISTANT/,
    "optimistic user messages should keep the copy action while the assistant reply is pending",
  );
  assert.match(
    messageBubbleSource,
    /<time dateTime=\{timestamp\} className="shrink-0 font-mono text-\[11px\] text-\[var\(--v2-text-muted\)\]">\{timeLabel\}<\/time>/,
    "timestamp should render in the hover meta row",
  );
  assert.match(
    messageBubbleSource,
    /mt-1 flex min-h-7 w-max v2-chat-readable-width flex-nowrap items-center gap-3 px-1 text-iron-400 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100/,
    "timestamp and controls should stay hidden until message hover or focus without being constrained to the bubble width",
  );
  assert.match(
    messageBubbleSource,
    /<div className="flex shrink-0 items-center gap-1">[\s\S]*<Icon name=\{copied \? "check" : "copy"\}/,
    "message actions should render in a non-shrinking group beside the timestamp",
  );

  const actionRow = messageBubbleSource.slice(
    messageBubbleSource.indexOf('"mt-1 flex min-h-7'),
    messageBubbleSource.indexOf("</div>", messageBubbleSource.indexOf('"mt-1 flex min-h-7')),
  );
  assert.doesNotMatch(
    actionRow,
    />\s*\$\{copied \? "Copied" : "Copy"\}\s*<|>Retry</,
    "hover controls should use fixed-size icons instead of text that competes with the timestamp",
  );
});

test("optimistic message opacity does not fade attached image previews", () => {
  assert.match(
    messageBubbleSource,
    /const contentOpacityClass = isOptimistic \? "opacity-70" : "";/,
    "optimistic pending state should dim only textual message content",
  );

  const contentBubbleClassArrayStart = messageBubbleSource.indexOf('"text-base leading-7"');
  const contentBubbleClassArray = messageBubbleSource.slice(
    contentBubbleClassArrayStart,
    messageBubbleSource.indexOf('].join(" ")}', contentBubbleClassArrayStart),
  );
  assert.doesNotMatch(
    contentBubbleClassArray,
    /isOptimistic|contentOpacityClass|opacity-70/,
    "the whole bubble must not be opacity-wrapped because attachments inherit that fade",
  );

  assert.match(
    messageBubbleSource,
    /images && images\.length > 0 && \([\s\S]*<img key=\{i\} src=\{src\} className="max-h-48 rounded-lg border border-iron-700 object-cover"/,
    "inline image previews should render outside the optimistic text opacity wrapper",
  );
  assert.match(
    messageBubbleSource,
    /attachments && attachments\.length > 0 && \([\s\S]*<AttachmentChip/,
    "attachment chips and thumbnails should render outside the optimistic text opacity wrapper",
  );
});
