// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";
import vm from "node:vm";

import { sourceTextForVmTest } from "../../test-support/vm-module-harness";

function evaluate(source, exportNames) {
  const context = { globalThis: {} };
  vm.runInNewContext(sourceTextForVmTest(source, exportNames), context);
  return context.globalThis.__testExports;
}

test("VM harness keeps code after semicolonless multiline imports", () => {
  const exports = evaluate(
    `
import {
  alpha,
  beta,
} from "./deps.js"
const value = "still here";
export function readValue() {
  return value;
}
`,
    ["readValue"]
  );

  assert.equal(exports.readValue(), "still here");
});

test("VM harness captures exported declarations and named export aliases", () => {
  const exports = evaluate(
    `
const base = 41;
export const answer = base + 1;
function readAnswer() {
  return answer;
}
export { readAnswer as getAnswer };
`,
    ["answer", "getAnswer"]
  );

  assert.equal(exports.answer, 42);
  assert.equal(exports.getAnswer(), 42);
});
