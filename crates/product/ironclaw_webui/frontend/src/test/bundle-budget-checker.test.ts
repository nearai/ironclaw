import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test } from "vitest";
import {
  createBundleAssetReader,
  resolveBundleAsset,
} from "../../scripts/check-bundle-budgets";

test("bundle asset reader caches emitted files after the first read", () => {
  const distDir = mkdtempSync(join(tmpdir(), "ironclaw-bundle-budget-"));
  const assetPath = join(distDir, "app.js");

  try {
    writeFileSync(assetPath, "first contents");
    const readAsset = createBundleAssetReader(distDir);
    const firstRead = readAsset("app.js");

    writeFileSync(assetPath, "changed contents");

    assert.strictEqual(readAsset("app.js"), firstRead);
    assert.equal(firstRead.toString(), "first contents");
  } finally {
    rmSync(distDir, { recursive: true, force: true });
  }
});

test("bundle asset paths cannot escape the build output directory", () => {
  const distDir = resolve(tmpdir(), "ironclaw-dist");

  assert.equal(
    resolveBundleAsset(distDir, "assets/app.js"),
    resolve(distDir, "assets/app.js"),
  );
  assert.throws(
    () => resolveBundleAsset(distDir, "../outside.js"),
    /escapes the dist directory/,
  );
  assert.throws(
    () => resolveBundleAsset(distDir, resolve(distDir, "../outside.js")),
    /escapes the dist directory/,
  );
});
