import assert from "node:assert/strict";
import { test } from "vitest";
import { resolveConnectExtension } from "./connect-extension";

test("resolves an installed extension by exact package ref id and returns the ConfigureModal shape", () => {
  const gmail = {
    package_ref: { id: "gmail" },
    display_name: "Gmail",
    installation_state: "active",
  };
  const resolved = resolveConnectExtension("gmail", [gmail], []);
  assert.notEqual(resolved, null);
  // ConfigureModal reads `packageRef` (not `package_ref`) and `displayName`.
  assert.deepEqual(resolved?.packageRef, { id: "gmail" });
  assert.equal(resolved?.displayName, "Gmail");
  // The raw extension fields are spread through so downstream schema checks
  // (channel surface, active state) still see the real record.
  assert.equal(resolved?.installation_state, "active");
});

test("tolerates separator/case drift between a demo app id and the real package ref", () => {
  // Demo cards carry "google_calendar"; the live package ref is "google-calendar".
  const calendar = { package_ref: { id: "google-calendar" }, display_name: "Google Calendar" };
  const resolved = resolveConnectExtension("google_calendar", [calendar], []);
  assert.deepEqual(resolved?.packageRef, { id: "google-calendar" });
});

test("prefers an installed extension over a registry entry with the same id", () => {
  const installed = { package_ref: { id: "gmail" }, display_name: "Gmail (installed)" };
  const registry = { package_ref: { id: "gmail" }, display_name: "Gmail (registry)" };
  const resolved = resolveConnectExtension("gmail", [installed], [registry]);
  assert.equal(resolved?.displayName, "Gmail (installed)");
});

test("falls back to a registry entry when nothing is installed yet", () => {
  const registry = { package_ref: { id: "notion" }, display_name: "Notion" };
  const resolved = resolveConnectExtension("notion", [], [registry]);
  assert.deepEqual(resolved?.packageRef, { id: "notion" });
});

test("returns null when no catalog entry plausibly matches — caller opens no modal", () => {
  const gmail = { package_ref: { id: "gmail" }, display_name: "Gmail" };
  assert.equal(resolveConnectExtension("dropbox", [gmail], []), null);
  // An empty catalog (offline / not yet loaded) also resolves to null rather
  // than throwing, so the surface degrades to a no-op Connect.
  assert.equal(resolveConnectExtension("gmail", [], []), null);
});

test("matches on display name when the package ref id is opaque", () => {
  const slack = { package_ref: { id: "ext_01H8XYZ" }, display_name: "Slack" };
  const resolved = resolveConnectExtension("slack", [slack], []);
  assert.deepEqual(resolved?.packageRef, { id: "ext_01H8XYZ" });
});
