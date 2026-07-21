import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";

const APP_CSS = readFileSync(new URL("./app.css", import.meta.url), "utf8");

test("stylesheet suppresses the typing dot under prefers-reduced-motion", () => {
  // Mirror of the Rust guard in webui_v2_serve.rs (PR #4493 contract): the
  // typing dot is the one intentional ambient loop and must opt out.
  assert.ok(
    APP_CSS.includes("@media (prefers-reduced-motion: reduce)"),
    "app.css must carry a reduced-motion block",
  );
  assert.ok(
    APP_CSS.includes(".v2-typing-dot { animation: none"),
    "the typing dot must be suppressed under prefers-reduced-motion: reduce",
  );
});
