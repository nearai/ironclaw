import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";

import { primaryRoutes } from "./routes";

test("Retired standalone Missions routes are absent from navigation and the app router", () => {
  assert.ok(
    !primaryRoutes.some((route) => route.id === "missions"),
    "Standalone Missions was superseded by trigger-backed Automations and must not remain routable",
  );
  assert.ok(
    !primaryRoutes.some(
      (route) => route.path === "/missions" || route.path === "/missions/:missionId",
    ),
    "Retired standalone Missions paths must not return under another route ID",
  );

  const appSource = readFileSync(new URL("./app.tsx", import.meta.url), "utf8");
  assert.doesNotMatch(
    appSource,
    /<Route\b[^>]*\bpath=["']missions(?:\/:missionId)?["']/s,
    "Neither /missions nor /missions/:missionId may remain registered",
  );
});
