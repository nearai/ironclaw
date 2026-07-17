import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, test } from "vitest";

import {
  checkSourceFile,
  checkSourceTree,
  formatViolation,
} from "../../scripts/check-source-conventions";

const temporaryRoots: string[] = [];

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) {
    rmSync(root, { force: true, recursive: true });
  }
});

test("rejects JavaScript-family source module extensions", () => {
  for (const extension of ["js", "jsx", "mjs", "cjs", "mts", "cts"]) {
    const violations = checkSourceFile(`feature.${extension}`, "export const value = 1;\n");

    assert.deepEqual(
      violations.map(({ kind }) => kind),
      ["invalid-module-extension"],
      `expected .${extension} to be rejected`,
    );
  }
});

test("rejects explicit extensions on every relative module import form", () => {
  const violations = checkSourceFile(
    "feature.ts",
    [
      'import value from "./value.ts";',
      'import "./side-effect.mjs";',
      'export { other } from "../other.js";',
      'const lazy = import("./lazy.tsx");',
      'const attributed = import("./data.ts", { with: { type: "json" } });',
    ].join("\n"),
  );

  assert.equal(violations.length, 5);
  assert.ok(violations.every(({ kind }) => kind === "explicit-relative-extension"));
  assert.deepEqual(
    violations.map(({ line }) => line),
    [1, 2, 3, 4, 5],
  );
});

test("rejects HTM html tagged templates", () => {
  const violations = checkSourceFile(
    "component.tsx",
    "const rendered = html`<div>Legacy</div>`;\n",
  );

  assert.deepEqual(violations.map(({ kind }) => kind), ["html-tagged-template"]);
});

test("allows TypeScript modules and non-module literal filenames", () => {
  const source = [
    'import React from "react";',
    'import { sibling } from "./sibling";',
    'export { nested } from "../nested";',
    'const lazy = import("./lazy");',
    'const fixture = new URL("./component.tsx", import.meta.url);',
    'const prose = "html`not syntax` and ./legacy.js";',
    "// html`also not syntax`",
  ].join("\n");

  assert.deepEqual(checkSourceFile("component.tsx", source), []);
  assert.deepEqual(checkSourceFile("utility.ts", source), []);
  assert.deepEqual(checkSourceFile("styles.css", "@layer base {}\n"), []);
});

test("recursively scans source trees and reports stable relative paths", () => {
  const root = mkdtempSync(join(tmpdir(), "ironclaw-source-conventions-"));
  temporaryRoots.push(root);
  mkdirSync(join(root, "nested"));
  writeFileSync(join(root, "nested", "legacy.js"), 'import "./other.ts";\n');
  writeFileSync(join(root, "valid.ts"), 'import "./valid";\n');

  const violations = checkSourceTree(root);

  assert.deepEqual(
    violations.map(({ file, kind, line }) => ({ file, kind, line })),
    [
      { file: "nested/legacy.js", kind: "explicit-relative-extension", line: 1 },
      { file: "nested/legacy.js", kind: "invalid-module-extension", line: 1 },
    ],
  );
  assert.equal(
    formatViolation(violations[0]),
    "nested/legacy.js:1: relative module imports must be extensionless",
  );
});
