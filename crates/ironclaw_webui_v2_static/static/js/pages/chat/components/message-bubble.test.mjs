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
    /mt-1 flex min-h-7 w-max max-w-\[85%\] flex-nowrap items-center gap-3 px-1 text-iron-400 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100/,
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

test("retry button is gated on a real onRetry handler and invokes it on click", () => {
  // The button must only render when `onRetry` is truthy — so callers that
  // have no retry handler pass null/undefined and the control hides. A
  // truthy no-op handler would satisfy this guard yet do nothing on click
  // (the dead-button regression this test exists to prevent).
  assert.match(
    messageBubbleSource,
    /const showRetryAction = status === "error" && onRetry;/,
    "retry button must be gated on a truthy onRetry handler, not rendered unconditionally",
  );
  // Clicking the button must actually call the handler with the message.
  assert.match(
    messageBubbleSource,
    /onClick=\$\{\(\)\s*=>\s*onRetry\(message\)\}/,
    "retry button click must invoke onRetry(message)",
  );
});
