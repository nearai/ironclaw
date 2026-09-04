// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";

function appSource() {
  return readFileSync(new URL("../../app/app.tsx", import.meta.url), "utf8");
}

test("a hub install deep link has a route to land on", () => {
  assert.ok(
    appSource().includes('path="install"'),
    "without this route an install deep link falls through to the catch-all and lands on chat",
  );
});

test("the install route requires a session", () => {
  const source = appSource();
  const layoutAt = source.indexOf("<AuthenticatedLayout");
  const installAt = source.indexOf('path="install"');

  assert.notEqual(layoutAt, -1);
  assert.notEqual(installAt, -1);
  assert.ok(
    installAt > layoutAt,
    "install must sit inside the authenticated layout so a signed-out caller cannot deliver one",
  );
});
