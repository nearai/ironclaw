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
