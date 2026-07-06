import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const messageBubbleSource = readFileSync(
  new URL("./message-bubble.js", import.meta.url),
  "utf8",
);
const appCssSource = readFileSync(
  new URL("../../../../styles/app.css", import.meta.url),
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
    /\.markdown-body\s*\{[^}]*overflow-wrap:\s*anywhere;[^}]*word-break:\s*break-word;/,
    "markdown prose should wrap long inline tokens on narrow screens",
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

test("conversation bubbles use mobile-safe widths and wrap long user tokens", () => {
  assert.match(
    messageBubbleSource,
    /\? "max-w-full sm:max-w-\[85%\]"/,
    "user bubbles should use full mobile width and restore narrower desktop width",
  );
  assert.match(
    messageBubbleSource,
    /: "w-full max-w-full sm:max-w-\[85%\]";/,
    "assistant bubbles should not exceed the mobile message column",
  );
  assert.match(
    messageBubbleSource,
    /className="v2-wrap-anywhere whitespace-pre-wrap break-words"/,
    "plain user text should break long unbroken strings",
  );
});

test("message timestamp and actions share a hover-only meta row", () => {
  assert.match(
    messageBubbleSource,
    /const showActions = role === "user" \|\| \(role === "assistant" && !isOptimistic\);/,
    "optimistic user messages should keep the copy action while the assistant reply is pending",
  );
  assert.match(
    messageBubbleSource,
    /<time dateTime=\$\{timestamp\} className="shrink-0 font-mono text-\[11px\] text-iron-500">\$\{timeLabel\}<\/time>/,
    "timestamp should render in the hover meta row",
  );
  assert.match(
    messageBubbleSource,
    /mt-1 flex min-h-7 w-max max-w-full flex-nowrap items-center gap-3 px-1 text-iron-400 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100 sm:max-w-\[85%\]/,
    "timestamp and controls should stay hidden until message hover or focus without being constrained to the bubble width",
  );
  assert.match(
    messageBubbleSource,
    /<div className="flex shrink-0 items-center gap-1">[\s\S]*<\$\{Icon\} name=\$\{copied \? "check" : "copy"\}/,
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
