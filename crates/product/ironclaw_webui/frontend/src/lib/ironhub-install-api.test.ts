// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import { installErrorKey, readIronhubInstallRequest } from "./ironhub-install-api";

const COMPLETE =
  "?slug=attio&version=0.1.0&uid=u1&aid=a1&ts=1750000000&nonce=n1" +
  "&artifact_digest=sha256%3Aabc&sig=deadbeef";

test("a complete deep link becomes the signed delivery request", () => {
  const { request, missing } = readIronhubInstallRequest(COMPLETE);

  assert.deepEqual(missing, []);
  assert.deepEqual(request, {
    slug: "attio",
    version: "0.1.0",
    uid: "u1",
    aid: "a1",
    ts: 1750000000,
    nonce: "n1",
    artifact_digest: "sha256:abc",
    sig: "deadbeef",
  });
});

test("ts is sent as a number so the signed payload keeps its type", () => {
  const { request } = readIronhubInstallRequest(COMPLETE);
  assert.equal(typeof request.ts, "number");
});

test("every signed field is required", () => {
  for (const field of [
    "slug",
    "version",
    "uid",
    "aid",
    "ts",
    "nonce",
    "artifact_digest",
    "sig",
  ]) {
    const params = new URLSearchParams(COMPLETE);
    params.delete(field);
    const { request, missing } = readIronhubInstallRequest(`?${params}`);

    assert.equal(request, null, `${field} missing must not produce a request`);
    assert.deepEqual(missing, [field]);
  }
});

test("a non-numeric timestamp is rejected rather than sent as NaN", () => {
  const params = new URLSearchParams(COMPLETE);
  params.set("ts", "not-a-number");
  const { request, missing } = readIronhubInstallRequest(`?${params}`);

  assert.equal(request, null);
  assert.deepEqual(missing, ["ts"]);
});

test("a private manifest url rides along when the hub supplies one", () => {
  const params = new URLSearchParams(COMPLETE);
  params.set("private_manifest_url", "https://hub.ironclaw.com/private/attio.json");
  const { request } = readIronhubInstallRequest(`?${params}`);

  assert.equal(request.private_manifest_url, "https://hub.ironclaw.com/private/attio.json");
});

test("the private manifest url is omitted entirely when absent", () => {
  const { request } = readIronhubInstallRequest(COMPLETE);
  assert.ok(
    !("private_manifest_url" in request),
    "deny_unknown_fields is off but a null would still be a wire change",
  );
});

test("a stale link is named as expired rather than a generic rejection", () => {
  const error = { status: 403, payload: { kind: "expired" } };
  assert.equal(installErrorKey(error), "ironhub.install.expired");
});

test("a replayed link is named as already used", () => {
  const error = { status: 403, payload: { kind: "duplicate" } };
  assert.equal(installErrorKey(error), "ironhub.install.alreadyUsed");
});

test("a bad signature stays the generic rejection", () => {
  const error = { status: 403, payload: { kind: "participant_denied" } };
  assert.equal(installErrorKey(error), "ironhub.install.rejected");
});

test("a 403 with no parsed payload still renders something legible", () => {
  assert.equal(installErrorKey({ status: 403 }), "ironhub.install.rejected");
});

test("a non-403 failure is not described as a link problem", () => {
  const error = { status: 500, payload: { kind: "internal" } };
  assert.equal(installErrorKey(error), "ironhub.install.failed");
});

test("an oversized field is rejected instead of reaching the consent card", () => {
  const search = COMPLETE.replace("slug=attio", `slug=${"a".repeat(129)}`);
  const { request, missing } = readIronhubInstallRequest(search);

  assert.equal(request, null);
  assert.deepEqual(missing, ["slug"]);
});

test("a slug outside the host charset is rejected", () => {
  const search = COMPLETE.replace("slug=attio", "slug=Attio%20Tool");
  const { request, missing } = readIronhubInstallRequest(search);

  assert.equal(request, null);
  assert.deepEqual(missing, ["slug"]);
});

test("a non-numeric timestamp is rejected rather than coerced", () => {
  for (const raw of ["1e3", "-1", "1.5", " 12 ", "abc"]) {
    const search = COMPLETE.replace(/ts=\d+/, `ts=${encodeURIComponent(raw)}`);
    const { request, missing } = readIronhubInstallRequest(search);

    assert.equal(request, null, `ts=${raw} must not parse`);
    assert.ok(missing.includes("ts"), `ts=${raw} must be named`);
  }
});

test("a non-http private manifest source is rejected", () => {
  for (const raw of ["javascript:alert(1)", "file:///etc/passwd", "not-a-url"]) {
    const search = `${COMPLETE}&private_manifest_url=${encodeURIComponent(raw)}`;
    const { request, missing } = readIronhubInstallRequest(search);

    assert.equal(request, null, `${raw} must not parse`);
    assert.ok(missing.includes("private_manifest_url"));
  }
});

test("every malformed field is named, not just the first", () => {
  const search = COMPLETE.replace("slug=attio", "slug=BAD").replace(/ts=\d+/, "ts=nope");
  const { missing } = readIronhubInstallRequest(search);

  assert.deepEqual(missing, ["slug", "ts"]);
});
