import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";

// Companion to the type-scale assertions in @ironclaw/ui's typography test:
// the root font-size and form-control font inheritance live in app.css and
// must stay viewport-independent (#6702).
test("root font-size and control font inheritance stay viewport-independent (#6702)", () => {
  const appCss = readFileSync(new URL("./app.css", import.meta.url), "utf8");

  assert.match(appCss, /html\s*\{[^}]*font-size:\s*16px;/s);
  assert.match(
    appCss,
    /@layer base\s*\{[^{}]*button, input, select, textarea\s*\{[^}]*font:\s*inherit;/s
  );
  assert.doesNotMatch(
    appCss,
    /@media\s*\(min-width:\s*1024px\)\s*\{[^}]*html\s*\{[^}]*font-size:/s
  );
});
