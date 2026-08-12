import assert from "node:assert/strict";
import { test } from "vitest";

import { primaryRoutes } from "./routes";

test("Retired Routines surface is absent from the route registry", () => {
  assert.ok(
    !primaryRoutes.some((route) => route.id === "routines"),
    "Routines was replaced by trigger-backed Automations and must not remain routable",
  );
});
