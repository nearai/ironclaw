import assert from "node:assert/strict";
import { test } from "vitest";

import { primaryRoutes } from "./routes";

test("Retired standalone Missions surface is absent from the route registry", () => {
  assert.ok(
    !primaryRoutes.some((route) => route.id === "missions"),
    "Standalone Missions was superseded by trigger-backed Automations and must not remain routable",
  );
});
