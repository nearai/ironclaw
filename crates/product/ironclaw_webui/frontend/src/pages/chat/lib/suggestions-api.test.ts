import assert from "node:assert/strict";
import { test } from "vitest";
import { formatSources, pollDelayMs } from "./suggestions-api";

test("formatSources joins human-readable tool names for the card provenance", () => {
  assert.equal(formatSources(["Gmail"]), "Gmail");
  assert.equal(formatSources(["Gmail", "Slack"]), "Gmail & Slack");
  assert.equal(formatSources(["Gmail", "Slack", "Telegram"]), "Gmail, Slack & Telegram");
});

test("formatSources returns empty string for missing/empty/blank sources", () => {
  assert.equal(formatSources([]), "");
  assert.equal(formatSources(null), "");
  assert.equal(formatSources(undefined), "");
  assert.equal(formatSources(["", "  "]), "");
});

// `pollDelayMs` is the only logic in the client worth pinning: the request
// builders are thin `apiFetch` wrappers, but this one turns an untrusted
// backend hint into a timer and must not produce a hot loop or a stall.

test("uses the backend's retry hint", () => {
  assert.equal(pollDelayMs({ status: "generating", retry_after_seconds: 3, suggestions: [] }), 3000);
});

test("falls back to 1s when the hint is missing, zero, or not a number", () => {
  // A missing hint is normal on a terminal status; 0/garbage would otherwise
  // schedule an immediate re-poll and spin the surface.
  assert.equal(pollDelayMs({ status: "generating", suggestions: [] }), 1000);
  assert.equal(pollDelayMs({ status: "generating", retry_after_seconds: 0, suggestions: [] }), 1000);
  assert.equal(
    pollDelayMs({ status: "generating", retry_after_seconds: NaN, suggestions: [] }),
    1000,
  );
  assert.equal(pollDelayMs(null), 1000);
});

test("clamps a hostile or absurd hint into a usable window", () => {
  // Negative would go backwards; a huge value would strand the user on the
  // generating state with no further polls.
  assert.equal(pollDelayMs({ status: "generating", retry_after_seconds: -5, suggestions: [] }), 1000);
  assert.equal(
    pollDelayMs({ status: "generating", retry_after_seconds: 86_400, suggestions: [] }),
    30_000,
  );
});
