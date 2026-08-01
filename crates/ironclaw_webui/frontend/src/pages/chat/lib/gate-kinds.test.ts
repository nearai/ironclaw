import assert from "node:assert/strict";
import { test } from "vitest";

import { GATE_KIND } from "./gate-kinds";

test("GATE_KIND exposes the wire discriminators", () => {
  assert.deepEqual({ ...GATE_KIND }, {
    APPROVAL: "approval",
    RESOURCE: "resource",
    GENERIC: "generic",
    AUTH: "auth",
  });
});

test("GATE_KIND is frozen so the discriminators cannot drift", () => {
  assert.ok(Object.isFrozen(GATE_KIND));
});
