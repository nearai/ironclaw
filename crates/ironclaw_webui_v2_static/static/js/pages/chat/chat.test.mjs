import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

// chat.js renders the full chat surface through htm templates, which the
// VM-context harness can't mount without a DOM. The retry *behaviour* is
// covered at the hook level (useChat.test.mjs) and the bubble level
// (message-bubble.test.mjs); this file guards the one seam those can't see:
// chat.js must route a landing-screen retry to the thread send() creates,
// exactly as handleSend does, or the new run attaches to a thread the user
// isn't viewing (useSSE is disabled while threadId is undefined).
const chatSource = readFileSync(new URL("./chat.js", import.meta.url), "utf8");

function handleRetryBlock() {
  const start = chatSource.indexOf("const handleRetry");
  assert.ok(start !== -1, "chat.js must define a handleRetry callback");
  const end = chatSource.indexOf("const handleSuggestion", start);
  return chatSource.slice(start, end === -1 ? undefined : end);
}

test("chat.js wires the Retry button to handleRetry, not the raw hook handler", () => {
  assert.match(
    chatSource,
    /onRetryMessage=\$\{handleRetry\}/,
    "MessageList's onRetryMessage must be handleRetry so retries get thread routing",
  );
});

test("handleRetry replays through retryMessage and routes a new thread like handleSend", () => {
  const block = handleRetryBlock();
  assert.match(
    block,
    /await retryMessage\(message\)/,
    "handleRetry must delegate the resend to the hook's retryMessage",
  );
  assert.match(
    block,
    /!activeThreadId &&[\s\S]*onSelectThread/,
    "handleRetry must call onSelectThread only when there is no active thread yet",
  );
});
