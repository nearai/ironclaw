import assert from "node:assert/strict";
import { test } from "vitest";
import { BRAND_ICON_IDS, resolveIconId } from "./brand-icons";

test("resolveIconId trusts the backend's required icon enum value", () => {
  assert.equal(resolveIconId({ icon: "slack" }), "slack");
  assert.equal(resolveIconId({ icon: "google_sheets" }), "google_sheets");
  assert.equal(resolveIconId({ icon: "generic" }), "generic");
});

test("resolveIconId falls back to generic for an out-of-enum or missing icon", () => {
  // The schema makes `icon` required + enum-constrained, but never render
  // nothing: an unexpected value (or an absent field on some future path)
  // degrades to the guaranteed-valid `generic` glyph.
  assert.equal(resolveIconId({ icon: "myspace" }), "generic");
  assert.equal(resolveIconId({}), "generic");
  assert.equal(resolveIconId({ icon: null }), "generic");
  assert.equal(resolveIconId(null), "generic");
});

test("every icon enum value is renderable (BrandIcon has a glyph for each)", () => {
  // Guards against the enum and the glyph map drifting: resolveIconId returns
  // exactly the value it was given for every enum member.
  for (const id of BRAND_ICON_IDS) {
    assert.equal(resolveIconId({ icon: id }), id);
  }
});

test("generic is a member of the enum (the guaranteed fallback)", () => {
  assert.ok(BRAND_ICON_IDS.includes("generic"));
});
