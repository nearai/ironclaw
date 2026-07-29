import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";

const APP_CSS = readFileSync(new URL("./app.css", import.meta.url), "utf8");
// The typing-dot loop moved into the design-system package with its
// TypingIndicator component; its reduced-motion opt-out lives there too.
const TOKENS_CSS = readFileSync(
  new URL("../../packages/design-system/src/tokens.css", import.meta.url),
  "utf8",
);

test("stylesheet suppresses the typing dot under prefers-reduced-motion", () => {
  // Mirror of the original Rust guard (PR #4493 contract): the typing dot
  // is the one intentional ambient loop and must opt out explicitly.
  assert.ok(
    APP_CSS.includes("@media (prefers-reduced-motion: reduce)"),
    "app.css must carry a blanket reduced-motion block",
  );
  assert.ok(
    TOKENS_CSS.includes(".v2-typing-dot { animation: none"),
    "the typing dot must be suppressed under prefers-reduced-motion: reduce",
  );
  assert.match(
    TOKENS_CSS,
    /@media \(prefers-reduced-motion: reduce\)\s*\{[^{}]*\.v2-typing-dot \{ animation: none/,
    "the typing-dot opt-out must sit inside a reduced-motion media block",
  );
});
