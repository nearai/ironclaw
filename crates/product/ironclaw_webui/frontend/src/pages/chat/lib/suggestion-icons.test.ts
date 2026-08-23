import assert from "node:assert/strict";
import { test } from "vitest";
import { SUGGESTION_ICON_IDS, resolveIconId } from "./suggestion-icons";

test("resolveIconId trusts the backend's required semantic icon value", () => {
  assert.equal(resolveIconId({ icon: "email" }), "email");
  assert.equal(resolveIconId({ icon: "spreadsheet" }), "spreadsheet");
  assert.equal(resolveIconId({ icon: "generic" }), "generic");
});

test("the icon vocabulary describes tasks without naming extensions", () => {
  assert.deepEqual(SUGGESTION_ICON_IDS, [
    "email",
    "calendar",
    "document",
    "storage",
    "spreadsheet",
    "presentation",
    "code",
    "messaging",
    "notes",
    "web",
    "memory",
    "generic",
  ]);
});

test("resolveIconId falls back to generic for an unknown or missing icon", () => {
  assert.equal(resolveIconId({ icon: "legacy_brand" }), "generic");
  assert.equal(resolveIconId({}), "generic");
  assert.equal(resolveIconId({ icon: null }), "generic");
  assert.equal(resolveIconId(null), "generic");
});

test("every icon enum value is renderable", () => {
  for (const id of SUGGESTION_ICON_IDS) {
    assert.equal(resolveIconId({ icon: id }), id);
  }
});

test("generic is a member of the enum", () => {
  assert.ok(SUGGESTION_ICON_IDS.includes("generic"));
});
