import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";

const toolActivitySource = readFileSync(
  new URL("./tool-activity.tsx", import.meta.url),
  "utf8",
);
const activityRunSource = readFileSync(
  new URL("./activity-run.tsx", import.meta.url),
  "utf8",
);
const typingIndicatorSource = readFileSync(
  new URL("./typing-indicator.tsx", import.meta.url),
  "utf8",
);

test("tool activity cards keep long tool output inside the mobile viewport", () => {
  assert.match(
    toolActivitySource,
    /className=\{nested \? "min-w-0 flex-1" : "min-w-0 flex-1 v2-chat-readable-width"\}/,
    "tool activity body should use full mobile width and constrain on larger screens",
  );
  assert.match(
    toolActivitySource,
    /className="min-w-0 overflow-hidden rounded-b-lg border-x border-b border-iron-700\/40 bg-iron-950"/,
    "tool detail panel should clip overflow to its card instead of widening the page",
  );
  assert.match(
    toolActivitySource,
    /const PRE_WRAP_CLASS =\s*\n\s*"v2-wrap-anywhere max-w-full overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono";/,
    "tool preformatted previews should share the mobile-safe wrapping class",
  );
  assert.equal(
    (
      toolActivitySource.match(
        /"v2-wrap-anywhere max-w-full overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono"/g,
      ) || []
    ).length,
    1,
    "tool preformatted preview class should be defined once",
  );
  assert.match(
    toolActivitySource,
    /<pre className=\{\[PRE_WRAP_CLASS, "text-iron-100"\]\.join\(" "\)\}>/,
    "tool parameters should wrap long lines within the detail panel",
  );
  assert.match(
    toolActivitySource,
    /PRE_WRAP_CLASS,[\s\S]*active === "declined" \? "text-iron-300" : "text-\[var\(--v2-danger-text\)\]"/,
    "tool errors should reuse the shared preformatted preview class",
  );
  assert.match(
    toolActivitySource,
    /className=\{\[PRE_WRAP_CLASS, "text-\[var\(--v2-positive-text\)\]"\]\.join\(" "\)\}/,
    "tool result previews should wrap long lines within the detail panel",
  );
  assert.match(
    toolActivitySource,
    /<div className="max-w-full overflow-x-auto rounded border border-iron-700\/60">/,
    "tool result tables should scroll horizontally inside the detail panel",
  );
  assert.doesNotMatch(
    toolActivitySource,
    /className="v2-wrap-anywhere border-b border-iron-700\/(?:60|40)/,
    "tool result table cells should keep natural column widths instead of aggressively wrapping",
  );
});

test("activity run wrappers use mobile-safe width constraints", () => {
  assert.match(
    activityRunSource,
    /className="mr-auto flex w-full min-w-0 flex-col v2-chat-readable-width"/,
    "activity run summary should use the shared readable width utility",
  );
  assert.match(
    activityRunSource,
    /className="min-w-0 flex-1 border-l-2 border-white\/10 pl-3 text-iron-300 v2-chat-readable-width"/,
    "reasoning inside activity runs should keep the same shared width",
  );
  assert.match(
    typingIndicatorSource,
    /className="flex min-w-0 flex-col gap-2 v2-chat-readable-width"/,
    "typing indicator should use the same shared readable width utility",
  );
  assert.doesNotMatch(
    [toolActivitySource, activityRunSource, typingIndicatorSource].join("\n"),
    /sm:max-w-\[[^\]]+\]/,
    "activity components should not scatter desktop width constants in component strings",
  );
});
