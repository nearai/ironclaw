// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import {
  runVmModuleForTest,
  sourceForVmTest,
} from "../../test-support/vm-module-harness";

const FIXTURE_PATH = "./vm-module-harness.fixture.ts";

function runFixture(exportNames) {
  return runVmModuleForTest(FIXTURE_PATH, exportNames, {}, import.meta.url);
}

test("VM harness keeps code after semicolonless multiline imports", () => {
  const exports = runFixture(["readValue"]);

  assert.equal(exports.readValue(), "still here");
});

test("VM harness captures exported declarations and named export aliases", () => {
  const exportNames = ["answer", "getAnswer"];
  const executableSource = sourceForVmTest(
    FIXTURE_PATH,
    exportNames,
    import.meta.url,
  );

  assert.doesNotMatch(executableSource, /^\s*(?:interface|type)\b/m);
  const exports = runFixture(exportNames);

  assert.equal(exports.answer, 42);
  assert.equal(exports.getAnswer(), 42);
});
