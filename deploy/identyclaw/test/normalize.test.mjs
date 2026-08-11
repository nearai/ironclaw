import assert from "node:assert/strict";
import test from "node:test";
import { hostKeyFromApiUrl, normalizeApiUrl } from "../src/lib.mjs";

test("normalizeApiUrl adds https and strips trailing slash", () => {
  assert.equal(normalizeApiUrl("api.identyclaw.com/"), "https://api.identyclaw.com");
  assert.equal(normalizeApiUrl("https://api-b.example.com/"), "https://api-b.example.com");
  assert.equal(normalizeApiUrl(""), "https://api.identyclaw.com");
});

test("hostKeyFromApiUrl is filesystem-safe", () => {
  assert.equal(hostKeyFromApiUrl("https://api.identyclaw.com"), "api.identyclaw.com");
  assert.equal(
    hostKeyFromApiUrl("https://slc.discernible.io:8443"),
    "slc.discernible.io_8443"
  );
});
