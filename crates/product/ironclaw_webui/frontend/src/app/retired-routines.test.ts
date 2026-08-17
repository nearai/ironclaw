import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "vitest";

import { primaryRoutes } from "./routes";

const SOURCE_ROOT = fileURLToPath(new URL("../", import.meta.url));
const RETIRED_ROUTINES_ROUTE =
  /<Route\b[^>]*\bpath=["']\/?routines(?:\/:routineId)?["']/s;

function productionSources(directory = SOURCE_ROOT): Array<[string, string]> {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      return productionSources(path);
    }
    if (!/\.[jt]sx?$/.test(entry.name) || /\.test\.[jt]sx?$/.test(entry.name)) {
      return [];
    }
    return [[path, readFileSync(path, "utf8")]];
  });
}

test("Retired Routines routes are absent from navigation and the app router", () => {
  assert.ok(
    !primaryRoutes.some((route) => route.id === "routines"),
    "Routines was replaced by trigger-backed Automations and must not remain routable",
  );

  const appSource = readFileSync(new URL("./app.tsx", import.meta.url), "utf8");
  assert.doesNotMatch(
    appSource,
    RETIRED_ROUTINES_ROUTE,
    "Neither /routines nor /routines/:routineId may remain registered",
  );
});

test("Retired Routines route guard recognizes nested and absolute paths", () => {
  for (const path of ["routines", "routines/:routineId", "/routines", "/routines/:routineId"]) {
    assert.match(
      `<Route path="${path}" element={<RoutinesPage />} />`,
      RETIRED_ROUTINES_ROUTE,
      `route guard must reject ${path}`,
    );
  }
});

test("Production source has no retired Routines page imports or links", () => {
  const violations = productionSources()
    .filter(
      ([, source]) =>
        /(?:from\s*|import\s*\()\s*["'][^"']*pages\/routines(?:\/[^"']*)?["']/.test(
          source,
        ) || /["'`]\/routines(?:\/[^"'`]*)?["'`]/.test(source),
    )
    .map(([path]) => path.slice(SOURCE_ROOT.length + 1));

  assert.deepEqual(
    violations,
    [],
    `Retired Routines imports or links remain in: ${violations.join(", ")}`,
  );
});
