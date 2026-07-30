// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { test, vi } from "vitest";

import {
  CHAT_MESSAGE_ROLES,
  type ErrorChatMessage,
} from "../lib/message-types";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

vi.mock("./markdown-renderer", async () => {
  const { createElement } = await import("react");
  return {
    MarkdownRenderer: ({ content, className, streaming }) =>
      createElement(
        "div",
        {
          className,
          "data-testid": "markdown",
          "data-streaming": String(Boolean(streaming)),
        },
        content,
      ),
  };
});

vi.mock("./tool-activity", async () => {
  const { createElement } = await import("react");
  return {
    ToolActivity: () => createElement("div", { "data-testid": "tool-activity" }),
  };
});

vi.mock("./command-result", async () => {
  const { createElement } = await import("react");
  return {
    CommandResult: ({ response, commands }) =>
      createElement("div", {
        "data-testid": "command-result-mock",
        "data-command": response?.command,
        "data-commands-count": Array.isArray(commands) ? commands.length : -1,
      }),
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
  resolve(process.cwd(), "src/pages/chat/components/message-bubble.tsx"),
  "utf8",
);
const appCssSource = readFileSync(
  resolve(process.cwd(), "src/styles/app.css"),
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

test("assistant bubbles keep streaming projections off the full markdown path", async () => {
  const { MessageBubble } = await import("./message-bubble");
  const render = (
    isFinalReply: boolean,
    activeRunId: string | null,
    isStreaming?: boolean,
  ) =>
    renderToStaticMarkup(
      React.createElement(MessageBubble, {
        message: {
          id: "assistant-stream",
          role: CHAT_MESSAGE_ROLES.ASSISTANT,
          content: '<img src=x onerror="alert(1)">',
          turnRunId: "run-1",
          isFinalReply,
          isStreaming,
        },
        activeRunId,
      }),
    );

  assert.match(render(false, "run-1"), /data-streaming="true"/);
  assert.match(render(true, "run-1"), /data-streaming="false"/);
  assert.match(
    render(false, null),
    /data-streaming="false"/,
    "historical assistant drafts must render through the completed Markdown path",
  );
  assert.match(
    render(false, "run-1", false),
    /data-streaming="false"/,
    "an earlier model phase must stop using the streaming Markdown path while the run continues",
  );
});

test("thinking bubbles defer Markdown only for the active run", async () => {
  const { MessageBubble } = await import("./message-bubble");
  const render = (activeRunId: string | null) =>
    renderToStaticMarkup(
      React.createElement(MessageBubble, {
        message: {
          id: "thinking-run-1",
          role: CHAT_MESSAGE_ROLES.THINKING,
          content: "Working on **this**.",
          turnRunId: "run-1",
        },
        activeRunId,
      }),
    );

  assert.match(render("run-1"), /data-streaming="true"/);
  assert.match(render(null), /data-streaming="false"/);
});

test("active reasoning in an activity run defers Markdown", async () => {
  const activity = [
    {
      id: "thinking-run-1",
      role: CHAT_MESSAGE_ROLES.THINKING,
      content: "Working on **this**.",
      turnRunId: "run-1",
    },
  ];

  assert.match(
    await renderExpandedActivity(activity, "run-1"),
    /data-streaming="true"/,
  );
  assert.match(
    await renderExpandedActivity(activity, null),
    /data-streaming="false"/,
  );
});

test("untagged reasoning in an activity run renders Markdown", async () => {
  const markup = await renderExpandedActivity([
    {
      id: "thinking-history",
      role: CHAT_MESSAGE_ROLES.THINKING,
      content: "Completed **reasoning**.",
    },
  ]);

  assert.match(markup, /data-streaming="false"/);
});

test("a SYSTEM message carrying a structured command result renders CommandResult, not the legacy markdown notice", async () => {
  const { MessageBubble } = await import("./message-bubble");
  const commands = [
    { name: "status", title: "Status", description: "d", usage: "/status" },
  ];
  // CommandResult is a React.lazy import behind a local Suspense boundary
  // (see message-bubble.tsx), so a synchronous renderToStaticMarkup would
  // only ever observe the fallback. Render into a real container instead and
  // await act() so the (mocked) dynamic import resolves before asserting —
  // the same pattern renderExpandedActivity() below uses.
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => {
      root.render(
        React.createElement(MessageBubble, {
          message: {
            id: "system-command-1",
            role: CHAT_MESSAGE_ROLES.SYSTEM,
            content: "**Status**\nState: idle",
            commandResult: {
              command: "status",
              result: { title: "Status", fields: [], lines: [] },
            },
            timestamp: "2026-07-30T00:00:00.000Z",
          },
          commands,
        }),
      );
    });
    const html = container.innerHTML;

    assert.match(html, /data-testid="command-result-mock"/);
    assert.match(html, /data-command="status"/, "CommandResult should receive the raw structured response");
    assert.match(
      html,
      /data-commands-count="1"/,
      "CommandResult should receive the server command inventory threaded down from chat.tsx",
    );
    assert.doesNotMatch(
      html,
      /data-testid="markdown"/,
      "a structured command result must not also render through the legacy markdown notice path",
    );
  } finally {
    act(() => root.unmount());
    container.remove();
  }
});

test("a plain SYSTEM notice without a structured command result keeps rendering through the legacy markdown bubble", async () => {
  const { MessageBubble } = await import("./message-bubble");
  const html = renderToStaticMarkup(
    React.createElement(MessageBubble, {
      message: {
        id: "system-busy-1",
        role: CHAT_MESSAGE_ROLES.SYSTEM,
        content: "The assistant is still working on the previous message.",
        timestamp: "2026-07-30T00:00:00.000Z",
      },
    }),
  );

  assert.match(
    html,
    /data-testid="markdown"/,
    "a plain system notice (e.g. the busy/rejected notice from send()) must keep its existing rendering",
  );
  assert.doesNotMatch(html, /data-testid="command-result-mock"/);
});

test("incoming reasoning and tool failures do not expand an activity run", async () => {
  const { ActivityRun } = await import("./activity-run");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const render = (activity) => {
    act(() => {
      root.render(React.createElement(ActivityRun, { activity }));
    });
    assert.equal(
      container
        .querySelector('[data-testid="activity-run-toggle"]')
        ?.getAttribute("aria-expanded"),
      "false",
    );
  };

  try {
    render([
      {
        id: "tool-search",
        role: CHAT_MESSAGE_ROLES.TOOL_ACTIVITY,
        toolName: "web-access.search",
        toolStatus: "running",
      },
    ]);
    render([
      {
        id: "reasoning",
        role: CHAT_MESSAGE_ROLES.THINKING,
        content: "Checking another source.",
      },
    ]);
    render([
      {
        id: "tool-search",
        role: CHAT_MESSAGE_ROLES.TOOL_ACTIVITY,
        toolName: "web-access.search",
        toolStatus: "error",
      },
    ]);
  } finally {
    act(() => root.unmount());
    container.remove();
  }
});

async function renderExpandedActivity(activity, activeRunId: string | null = null) {
  const { ActivityRun } = await import("./activity-run");
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);

  try {
    act(() => {
      root.render(React.createElement(ActivityRun, { activity, activeRunId }));
    });
    const toggle = container.querySelector<HTMLButtonElement>(
      '[data-testid="activity-run-toggle"]',
    );
    assert.equal(toggle?.getAttribute("aria-expanded"), "false");
    act(() => toggle?.click());
    assert.equal(toggle?.getAttribute("aria-expanded"), "true");
    return container.innerHTML;
  } finally {
    act(() => root.unmount());
    container.remove();
  }
}

test("only final assistant replies expose the run artifact download", async () => {
  const { MessageBubble } = await import("./message-bubble");
  const render = (isFinalReply: boolean) =>
    renderToStaticMarkup(
      React.createElement(MessageBubble, {
        message: {
          id: "assistant-1",
          role: CHAT_MESSAGE_ROLES.ASSISTANT,
          content: "done",
          timestamp: "2026-06-02T00:00:00.000Z",
          turnRunId: "run-1",
          isFinalReply,
        },
        threadId: "thread-1",
      }),
    );

  assert.match(render(true), /data-testid="download-run-artifact"/);
  assert.doesNotMatch(render(false), /data-testid="download-run-artifact"/);
  assert.match(messageBubbleSource, /fetchRunArtifact\(\{/);
  assert.match(messageBubbleSource, /ironclaw-run-\$\{filenameRunId\}\.json/);
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

test("intermediate assistant phases do not reserve a hidden meta row", async () => {
  const { MessageBubble } = await import("./message-bubble");
  const html = renderToStaticMarkup(
    React.createElement(MessageBubble, {
      message: {
        id: "assistant-phase",
        role: CHAT_MESSAGE_ROLES.ASSISTANT,
        content: "I will check that.",
        timestamp: "2026-07-30T00:00:00.000Z",
        turnRunId: "run-1",
        isFinalReply: false,
        isStreaming: false,
      },
      activeRunId: "run-1",
    }),
  );

  assert.doesNotMatch(
    html,
    /<time|chat\.copyMessage/,
    "intermediate utterances should not add invisible controls between tool runs",
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
