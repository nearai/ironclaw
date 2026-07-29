import assert from "node:assert/strict";
import { readFileSync, readdirSync, realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

import { test } from "vitest";

const FRONTEND = path.dirname(
  fileURLToPath(new URL("../../../package.json", import.meta.url)),
);
const PNPM_STORE = path.join(FRONTEND, "node_modules", ".pnpm");

/**
 * The Ledger WebHID transport calls `captureException` on every device error
 * path. In the real `@sentry/minimal` those calls delegate to whatever Sentry
 * hub is current — a live route out of the process, inside a
 * transaction-signing bundle, armed the moment anything on the page
 * initializes Sentry.
 *
 * A pnpm override aliases it to a local no-op. These tests are the guard: an
 * override is one line in a config file that a dependency bump or a lockfile
 * regeneration can quietly drop, and nothing else in the suite would notice.
 */
test("every Ledger package resolves @sentry/minimal to the no-op stub", () => {
  const consumers = readdirSync(PNPM_STORE).filter((entry) =>
    entry.startsWith("@ledgerhq+"),
  );
  assert.ok(consumers.length > 0, "the Ledger packages must be installed");

  const checked: string[] = [];
  for (const consumer of consumers) {
    const link = path.join(PNPM_STORE, consumer, "node_modules", "@sentry", "minimal");
    let target: string;
    try {
      target = realpathSync(link);
    } catch {
      continue; // this package does not depend on it
    }
    checked.push(consumer);
    assert.match(
      target,
      /sentry-minimal-noop/,
      `${consumer} resolved a real Sentry SDK at ${target}; the pnpm override ` +
        "in pnpm-workspace.yaml has been lost",
    );
  }

  assert.ok(
    checked.length > 0,
    "no Ledger package linked @sentry/minimal at all — if Ledger dropped the " +
      "dependency, remove the override and this test; do not leave it passing vacuously",
  );
});

/** The lockfile is the other place the override can silently disappear. */
test("no package in the lockfile resolves a real @sentry/minimal", () => {
  const lock = readFileSync(path.join(FRONTEND, "pnpm-lock.yaml"), "utf8");
  const realResolutions = lock
    .split("\n")
    .filter((line) => line.includes("'@sentry/minimal':"))
    .filter((line) => !line.includes("noop"));

  assert.deepEqual(realResolutions, [], "a real @sentry/minimal is still resolved");
});

/** And the stub must be genuinely inert, not merely present. */
test("the stub captures nothing and returns a usable event id", async () => {
  const stub = await import(
    path.join(FRONTEND, "vendor", "sentry-minimal-noop", "index.js")
  );
  assert.equal(typeof stub.captureException, "function");
  assert.equal(
    stub.captureException(new Error("device exploded")),
    "",
    "captureException must swallow and return an event id shape",
  );
});
