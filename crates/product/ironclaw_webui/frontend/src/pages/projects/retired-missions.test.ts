import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";

function readSource(path: string) {
  return readFileSync(new URL(path, import.meta.url), "utf8");
}

test("Projects does not expose the retired Mission API or nested route", () => {
  const apiSource = readSource("./lib/projects-api.ts");
  const appSource = readSource("../../app/app.tsx");

  assert.doesNotMatch(apiSource, /fetchProjectMissions|fetchMissionDetail|fireMission|pauseMission|resumeMission/);
  assert.doesNotMatch(appSource, /projects\/:projectId\/missions\/:missionId/);
});
