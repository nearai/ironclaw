import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const toolActivitySource = readFileSync(
  new URL("./tool-activity.js", import.meta.url),
  "utf8",
);
const activityRunSource = readFileSync(
  new URL("./activity-run.js", import.meta.url),
  "utf8",
);

test("tool activity cards keep long tool output inside the mobile viewport", () => {
  assert.match(
    toolActivitySource,
    /className=\$\{nested \? "min-w-0 flex-1" : "min-w-0 max-w-full flex-1 sm:max-w-\[85%\]"\}/,
    "tool activity body should use full mobile width and constrain on larger screens",
  );
  assert.match(
    toolActivitySource,
    /className="min-w-0 overflow-hidden rounded-b-lg border-x border-b border-iron-700\/40 bg-iron-950"/,
    "tool detail panel should clip overflow to its card instead of widening the page",
  );
  assert.match(
    toolActivitySource,
    /className="v2-wrap-anywhere max-w-full overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-iron-100"/,
    "tool parameters should wrap long lines within the detail panel",
  );
  assert.match(
    toolActivitySource,
    /className="v2-wrap-anywhere max-w-full overflow-x-auto whitespace-pre-wrap rounded bg-iron-900 p-2 font-mono text-\[var\(--v2-positive-text\)\]"/,
    "tool result previews should wrap long lines within the detail panel",
  );
});

test("activity run wrappers use mobile-safe width constraints", () => {
  assert.match(
    activityRunSource,
    /className="mr-auto flex w-full min-w-0 max-w-full flex-col sm:max-w-\[85%\]"/,
    "activity run summary should not exceed the mobile message column",
  );
  assert.match(
    activityRunSource,
    /className="min-w-0 max-w-full flex-1 border-l-2 border-white\/10 pl-3 text-iron-300 sm:max-w-\[85%\]"/,
    "reasoning inside activity runs should keep the same mobile-safe width",
  );
});
