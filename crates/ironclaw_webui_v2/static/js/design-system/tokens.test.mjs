import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  COLOR_TOKENS,
  CONTROL_TOKENS,
  MOTION_TOKENS,
  RADIUS_TOKENS,
  SHADOW_TOKENS,
  SPACE_TOKENS,
  STATUS_CANON,
  TYPE_TOKENS,
  Z_TOKENS,
} from "./tokens.js";

// tokens.js is the machine-readable *index* of the design system; the
// canonical *values* live in app.css. These tests pin the contract between
// the two: every catalogued token must resolve at runtime (a stale catalog
// entry renders as an empty var() — invisible breakage), and the catalog
// must not drift into duplicates.

const APP_CSS = readFileSync(
  new URL("../../styles/app.css", import.meta.url),
  "utf8",
);

/** Custom-property names defined anywhere in app.css (either theme). */
const DEFINED = new Set(
  [...APP_CSS.matchAll(/(--v2-[a-z0-9-]+)\s*:/g)].map((m) => m[1]),
);

function allCataloguedVars() {
  const flat = [];
  for (const group of COLOR_TOKENS) {
    for (const token of group.tokens) flat.push(token.var);
  }
  for (const list of [
    CONTROL_TOKENS,
    RADIUS_TOKENS,
    SPACE_TOKENS,
    TYPE_TOKENS,
    SHADOW_TOKENS,
    MOTION_TOKENS,
    Z_TOKENS,
  ]) {
    for (const token of list) flat.push(token.var);
  }
  return flat;
}

test("every catalogued token is defined in app.css", () => {
  const missing = allCataloguedVars().filter((name) => !DEFINED.has(name));
  assert.deepEqual(
    missing,
    [],
    "tokens.js lists custom properties that app.css never defines — a stale catalog entry resolves var() to nothing at runtime",
  );
});

test("the catalog holds no duplicate token entries", () => {
  const vars = allCataloguedVars();
  const dupes = vars.filter((name, i) => vars.indexOf(name) !== i);
  assert.deepEqual(dupes, [], "each token must be catalogued exactly once");
});

test("status canon references only defined tokens", () => {
  for (const entry of STATUS_CANON) {
    assert.ok(
      DEFINED.has(entry.text),
      `status "${entry.status}" text token ${entry.text} missing from app.css`,
    );
    assert.ok(
      DEFINED.has(entry.fill),
      `status "${entry.status}" fill token ${entry.fill} missing from app.css`,
    );
  }
});

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
