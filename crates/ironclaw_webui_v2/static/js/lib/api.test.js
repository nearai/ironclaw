// Unit tests for the WebChat v2 API error-message derivation.
//
// Run with Node's built-in test runner (no extra deps):
//   node --test crates/ironclaw_webui_v2/static/js/lib/api.test.js
//
// NOTE: `build.rs` deliberately excludes `*.test.js` from the embedded
// asset bundle, so this file is never served to the browser.

import assert from "node:assert/strict";
import { test } from "node:test";
import { describeApiError } from "./api.js";

test("structured error envelope humanizes the user-renderable kind, not raw JSON", () => {
  // Regression for the NEAR AI save bug: a 503 returned
  // {"error":"unavailable","kind":"service_unavailable",...}. Surfacing the
  // raw JSON body read as "no error message"; the user must see readable text.
  assert.equal(
    describeApiError({
      payload: { error: "unavailable", kind: "service_unavailable", retryable: false },
      body: '{"error":"unavailable","kind":"service_unavailable","retryable":false}',
      statusText: "Service Unavailable",
    }),
    "Service unavailable"
  );
});

test("validation errors prefer the specific code and name the field", () => {
  assert.equal(
    describeApiError({
      payload: { error: "invalid_request", kind: "validation", validation_code: "invalid_id", field: "provider_id" },
      body: "{...}",
      statusText: "Bad Request",
    }),
    "Invalid id (provider_id)"
  );
});

test("a short non-JSON body is surfaced as-is", () => {
  assert.equal(
    describeApiError({ payload: undefined, body: "upstream timeout", statusText: "Bad Gateway" }),
    "upstream timeout"
  );
});

test("a raw JSON blob without a usable code falls back to status text, never the blob", () => {
  assert.equal(
    describeApiError({
      payload: { unexpected: true },
      body: '{"unexpected":true}',
      statusText: "Internal Server Error",
    }),
    "Internal Server Error"
  );
});

test("an empty body falls back to status text", () => {
  assert.equal(
    describeApiError({ payload: undefined, body: "", statusText: "Service Unavailable" }),
    "Service Unavailable"
  );
});

test("with nothing usable at all, a generic message is returned", () => {
  assert.equal(describeApiError({}), "Request failed");
});
