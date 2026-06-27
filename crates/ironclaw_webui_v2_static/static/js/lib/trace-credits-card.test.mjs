import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  formatSignedCredit,
  sidebarTraceCreditsSummary,
} from "./trace-credits-card.js";

function source(relativePath) {
  return readFileSync(new URL(relativePath, import.meta.url), "utf8");
}

function assertIncludes(haystack, needles, label) {
  for (const needle of needles) {
    assert.ok(
      haystack.includes(needle),
      `${label} should include ${JSON.stringify(needle)}`
    );
  }
}

test("formatSignedCredit renders signed two-decimal credit summaries", () => {
  assert.equal(formatSignedCredit(12), "+12.00");
  assert.equal(formatSignedCredit(2.345), "+2.35");
  assert.equal(formatSignedCredit(0), "+0.00");
  assert.equal(formatSignedCredit(""), "+0.00");
  assert.equal(formatSignedCredit(undefined), "+0.00");
  assert.equal(formatSignedCredit(Number.NaN), "+0.00");
  assert.equal(formatSignedCredit(-4.5), "-4.50");
});

test("sidebarTraceCreditsSummary hides loading, error, and not-enrolled states", () => {
  assert.equal(sidebarTraceCreditsSummary(null), null);
  assert.equal(sidebarTraceCreditsSummary(undefined), null);
  assert.equal(sidebarTraceCreditsSummary({ enrolled: false }), null);
});

test("sidebarTraceCreditsSummary defaults absent counts and preserves positive hold counts", () => {
  assert.deepEqual(
    sidebarTraceCreditsSummary({
      enrolled: true,
      final_credit: "7.1",
      submissions_accepted: 3,
      submissions_submitted: 5,
      manual_review_hold_count: 2,
    }),
    {
      final: "+7.10",
      accepted: 3,
      submitted: 5,
      heldCount: 2,
    }
  );

  assert.deepEqual(
    sidebarTraceCreditsSummary({
      enrolled: true,
      final_credit: undefined,
      manual_review_hold_count: 0,
    }),
    {
      final: "+0.00",
      accepted: 0,
      submitted: 0,
      heldCount: 0,
    }
  );
});

test("SidebarTraceCredits remains display-only and links to the Trace Commons tab", () => {
  const card = source("../components/sidebar-trace-credits.js");
  const sidebar = source("../components/sidebar.js");
  const hook = source("../pages/settings/hooks/useTraceCredits.js");

  assertIncludes(
    card,
    [
      'import { sidebarTraceCreditsSummary } from "../lib/trace-credits-card.js";',
      "useTraceCredits()",
      "const summary = sidebarTraceCreditsSummary(credits);",
      "if (!summary) return null;",
      'to="/settings/traces"',
      "traceCommons.finalCredit",
      "traceCommons.cardAccepted",
      "heldCount > 0",
      "traceCommons.cardHeld",
    ],
    "SidebarTraceCredits"
  );

  assertIncludes(
    sidebar,
    [
      'import { SidebarTraceCredits } from "./sidebar-trace-credits.js";',
      "<${SidebarTraceCredits} />",
      "<${SidebarThreads}",
    ],
    "Sidebar"
  );

  assertIncludes(
    hook,
    [
      'queryKey: ["trace-credits"]',
      "queryFn: fetchTraceCredits",
      "refetchInterval: 300_000",
      "refetchIntervalInBackground: false",
      "refetchOnWindowFocus: true",
      "staleTime: 60_000",
    ],
    "useTraceCredits"
  );
});
