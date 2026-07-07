import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";
import vm from "node:vm";

import { sourceForTest } from "./source-for-test.mjs";

test("sourceForTest strips multiline imports and common export forms", async () => {
  const dir = mkdtempSync(join(tmpdir(), "source-for-test-"));
  writeFileSync(
    join(dir, "fixture.js"),
    `
import {
  unused
} from "./unused.js";

export const value = 42;
export class Thing {
  label() {
    return "thing";
  }
}
export async function loadValue() {
  return value;
}
export default { ok: true };
export {
  value as renamedValue,
  Thing as RenamedThing
};
`
  );

  const context = { globalThis: {} };
  const baseUrl = pathToFileURL(join(dir, "source-for-test.test.mjs")).href;

  vm.runInNewContext(
    sourceForTest(baseUrl, "./fixture.js", ["value", "Thing", "loadValue", "default"]),
    context
  );

  const exports = context.globalThis.__testExports;
  assert.equal(exports.value, 42);
  assert.equal(new exports.Thing().label(), "thing");
  assert.equal(await exports.loadValue(), 42);
  assert.equal(exports.default.ok, true);
});

test("sourceForTest maps named default function and class exports", () => {
  const dir = mkdtempSync(join(tmpdir(), "source-for-test-"));
  writeFileSync(
    join(dir, "default-function.js"),
    `
export default function namedDefault() {
  return namedDefault.name;
}
`
  );
  writeFileSync(
    join(dir, "default-class.js"),
    `
export default class NamedDefault {
  label() {
    return NamedDefault.name;
  }
}
`
  );
  const baseUrl = pathToFileURL(join(dir, "source-for-test.test.mjs")).href;

  const functionContext = { globalThis: {} };
  vm.runInNewContext(sourceForTest(baseUrl, "./default-function.js", ["default"]), functionContext);
  assert.equal(functionContext.globalThis.__testExports.default(), "namedDefault");

  const classContext = { globalThis: {} };
  vm.runInNewContext(sourceForTest(baseUrl, "./default-class.js", ["default"]), classContext);
  assert.equal(new classContext.globalThis.__testExports.default().label(), "NamedDefault");
});
