import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { test } from "vitest";

const SRC_ROOT = fileURLToPath(new URL("../", import.meta.url));

function productionTypeScriptFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = `${directory}/${entry.name}`;
    if (entry.isDirectory()) return productionTypeScriptFiles(path);
    if (!/\.tsx?$/.test(entry.name) || /\.test\.tsx?$/.test(entry.name)) return [];
    return [path];
  });
}

test("semantic states and secondary text use theme-aware colors (#6039)", () => {
  const violations: string[] = [];

  for (const path of productionTypeScriptFiles(SRC_ROOT)) {
    const source = readFileSync(path, "utf8");
    if (/\b(?:bg|border|text)-emerald-\d+/.test(source)) {
      violations.push(`${path}: fixed emerald utility`);
    }
    if (/\biron-(?:500|600)\b/.test(source)) {
      violations.push(`${path}: undefined iron utility`);
    }
  }

  assert.deepEqual(violations, []);
});
