import assert from "node:assert/strict";
import { test } from "vitest";
import { BRAND_ICON_IDS, iconIdForSource, resolveIconId } from "./brand-icons";

test("maps extension package ids to brand icons", () => {
  assert.equal(iconIdForSource("gmail"), "gmail");
  assert.equal(iconIdForSource("google-calendar"), "google_calendar");
  assert.equal(iconIdForSource("google-sheets"), "google_sheets");
  assert.equal(iconIdForSource("notion-mcp"), "notion");
  assert.equal(iconIdForSource("web-app"), "web");
  assert.equal(iconIdForSource("memory-native"), "memory");
});

test("unknown, empty, or missing source ids fall back to generic", () => {
  assert.equal(iconIdForSource("dropbox"), "generic");
  assert.equal(iconIdForSource(""), "generic");
  assert.equal(iconIdForSource(null), "generic");
  assert.equal(iconIdForSource(undefined), "generic");
});

test("resolveIconId prefers an explicit valid icon enum value", () => {
  assert.equal(resolveIconId({ icon: "slack", source_ids: ["gmail"] }), "slack");
});

test("resolveIconId ignores an unknown icon value and derives from source_ids", () => {
  // A model could emit an out-of-enum icon; never trust it blindly — fall back
  // to the (trusted) source id derivation rather than rendering nothing.
  assert.equal(resolveIconId({ icon: "myspace", source_ids: ["github"] }), "github");
});

test("resolveIconId derives from the first source id when no explicit icon", () => {
  assert.equal(resolveIconId({ source_ids: ["telegram", "gmail"] }), "telegram");
});

test("resolveIconId falls back to generic for a tool-less suggestion", () => {
  // e.g. "draft a project plan" — no tool. The enum's guaranteed value keeps
  // the card renderable before the backend even sends the field.
  assert.equal(resolveIconId({}), "generic");
  assert.equal(resolveIconId(null), "generic");
  assert.equal(resolveIconId({ source_ids: [] }), "generic");
});

test("generic is a member of the enum (the guaranteed fallback)", () => {
  assert.ok(BRAND_ICON_IDS.includes("generic"));
});
